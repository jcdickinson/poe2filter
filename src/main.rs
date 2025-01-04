use std::{
    collections::{HashMap, HashSet, VecDeque},
    env::{args_os, var_os},
    ffi::{CString, OsStr, OsString},
    io::{Cursor, Read},
    os::unix::prelude::OsStrExt,
    path::{Path, PathBuf},
};

use color_eyre::{
    eyre::{bail, eyre, Context},
    Result,
};
use log::{debug, error, info, warn};
use reqwest::{header::HeaderValue, Client, ClientBuilder};
use serde::Deserialize;
use tokio::{fs, io::AsyncWriteExt};

#[derive(Debug, Clone, Deserialize)]
struct ReleaseInfo {
    zipball_url: String,
    tag_name: String,
    body: Option<String>,
}

#[derive(Debug, Clone)]
struct Globals {
    game_directory: PathBuf,
    versions: HashMap<String, String>,
    client: Client,
}

impl Globals {
    async fn new() -> Result<Self> {
        let game_directory = locate_game_directory()
            .await
            .wrap_err_with(|| "could not find game directory")?;

        let client = ClientBuilder::new()
            .user_agent("poe2filter")
            .build()
            .wrap_err_with(|| "could not create an HTTP client")?;

        let mut versions = HashMap::default();
        if let Ok(store) = fs::read_to_string(releases_file(&game_directory)).await {
            if let Ok(existing_versions) = serde_json::from_str(&store).inspect_err(|error| {
                error!("could not read existing files, starting from scratch: {error}")
            }) {
                versions = existing_versions;
            }
        }

        Ok(Globals {
            game_directory,
            versions,
            client,
        })
    }
}

fn main() -> Result<()> {
    pretty_env_logger::init_custom_env("POE2FILTER_LOG");

    let sep = OsString::from("--");
    let mut args: VecDeque<_> = args_os().collect();

    debug!("args are {args:?}");
    args.pop_front(); // Remove "poe2filter"

    let mut sources = Vec::new();
    while let Some(front) = args.pop_front() {
        if front == sep {
            break;
        }
        sources.push(front);
    }

    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("spawn async runtime");

        rt.block_on(async_main(sources))?;
    }

    let Some(path) = args.front().cloned() else {
        info!("nothing to execute provided");
        return Ok(());
    };

    let args: Vec<_> = args.iter().map(|v| to_cstr(v.as_os_str())).collect();

    info!("starting {path:?} {args:?}");
    nix::unistd::execv(&to_cstr(&path), &args)?;

    Ok(())
}

async fn async_main(sources: Vec<OsString>) -> Result<()> {
    let mut globals = Globals::new().await?;

    for source in sources {
        let source = source
            .to_str()
            .ok_or_else(|| eyre!("all arguments must be valid UTF-8"))?;

        let source = match source {
            "neversink-lite" => "github:NeverSinkDev/NeverSink-PoE2litefilter",
            "cdrg" => "github:cdrg/cdr-poe2filter",
            other => other,
        };

        let index = source
            .find(':')
            .ok_or_else(|| eyre!("all arguments must be in the form source:arg"))?;
        let (source_name, value) = source.split_at(index);

        let current_version = globals.versions.get(source);
        info!(
            "updating {source} which has watermark {}...",
            current_version.map(|v| v.as_str()).unwrap_or("none")
        );
        let next_version = match source_name {
            "github" => get_github(&globals, &value[1..], current_version).await?,
            _ => bail!("source type must be github"),
        };

        info!("watermark for {source} set to {next_version}");
        globals.versions.insert(source.to_string(), next_version);
    }

    info!("saving watermark");
    let s = serde_json::to_string_pretty(&globals.versions)?;
    let mut o = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&releases_file(&globals.game_directory))
        .await?;
    o.write_all(s.as_bytes()).await?;

    info!("saved watermark");
    Ok(())
}

async fn get_github(globals: &Globals, value: &str, existing: Option<&String>) -> Result<String> {
    static API_VERSION: HeaderValue = HeaderValue::from_static("2022-11-28");
    static API_JSON_TYPE: HeaderValue = HeaderValue::from_static("application/vnd.github+json");

    info!("fetching latest release");
    let releases = globals
        .client
        .get(format!(
            "https://api.github.com/repos/{value}/releases?per_page=1&page=0"
        ))
        .header("X-Github-Api-Version", API_VERSION.clone())
        .header("Accept", API_JSON_TYPE.clone())
        .send()
        .await?
        .error_for_status()?
        .json::<Vec<ReleaseInfo>>()
        .await?;

    let release = releases
        .into_iter()
        .next()
        .ok_or_else(|| eyre!("no release could be found"))?;

    info!("found release with tag: {}", release.tag_name);

    if existing == Some(&release.tag_name) {
        info!("source is up to date");
        return Ok(release.tag_name);
    }

    eprintln!("# github:{value}: {}", &release.tag_name);
    if let Some(body) = release.body.as_ref() {
        eprintln!("{body}");
    }
    eprintln!();

    info!("downloading release zipball");
    let zipball = globals
        .client
        .get(&release.zipball_url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?
        .to_vec();

    info!("opening release zipball");
    let mut zipfile = zip::ZipArchive::new(Cursor::new(zipball))?;
    let filter = OsString::from("filter");
    let filenames: Vec<_> = zipfile.file_names().map(|v| v.to_string()).collect();
    let mut file_data = Vec::new();

    for filename in filenames {
        let path = PathBuf::from(&filename);
        if Some(filter.as_os_str()) != path.extension() {
            continue;
        }

        info!("extracting {filename}");
        let mut file = zipfile.by_name(&filename)?;
        file_data.clear();
        file.read_to_end(&mut file_data)?;

        let Some(filename) = PathBuf::from(&filename)
            .file_name()
            .map(|v| v.to_os_string())
        else {
            // Not really possible, but avoid panicking
            continue;
        };

        let full_path = globals.game_directory.join(&filename);

        info!("writing {full_path:?}");
        let mut dest = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(full_path)
            .await?;

        dest.write_all(&file_data).await?;
    }

    info!("updated github:{value}");

    Ok(release.tag_name)
}

fn split_paths(raw: OsString) -> Vec<PathBuf> {
    if raw.is_empty() {
        return Default::default();
    }

    let mut bytes = raw.as_bytes();
    let mut result = Vec::new();

    while !bytes.is_empty() {
        let index = bytes
            .iter()
            .cloned()
            .enumerate()
            .find_map(|(idx, v)| if v == b':' { Some(idx) } else { None })
            .unwrap_or(bytes.len());

        let (current, next) = bytes.split_at(index);
        if next.is_empty() {
            break;
        }
        bytes = &next[1..]; // Remove the :
        result.push(PathBuf::from(OsStr::from_bytes(current)));
    }

    result
}

async fn locate_game_directory() -> Result<PathBuf> {
    let mut paths = Vec::new();

    if let Some(compat_path) = var_os("STEAM_COMPAT_DATA_PATH") {
        paths.push(PathBuf::from(compat_path));
    }

    let game_id = var_os("STEAM_COMPAT_APP_ID")
        .or_else(|| var_os("SteamGameId"))
        .unwrap_or_else(|| OsString::from("2694490"));

    if let Some(compat_paths) = var_os("STEAM_COMPAT_LIBRARY_PATHS") {
        for path in split_paths(compat_paths) {
            paths.push(path.join("compatdata").join(&game_id));
        }
    }

    if let Some(base_path) = var_os("STEAM_BASE_FOLDER") {
        let base_path = PathBuf::from(base_path);
        paths.push(base_path.join("steamapps/compatdata").join(&game_id));
    }

    if let Some(data_dirs) = var_os("XDG_DATA_DIRS") {
        for path in split_paths(data_dirs) {
            paths.push(path.join("Steam/steamapps/compatdata").join(&game_id));
        }
    }

    if let Some(home) = var_os("HOME") {
        let home = PathBuf::from(home);
        paths.push(
            home.join(".local/share/Steam/steamapps/compatdata")
                .join(&game_id),
        );
    }

    let mut checked_paths = HashSet::new();
    for path in paths
        .into_iter()
        .filter(|v| checked_paths.insert(v.clone()))
    {
        let path = path.join("pfx/drive_c/users/steamuser/My Documents/My Games");
        info!("checking {path:?}...");
        if let Ok(true) = fs::try_exists(&path).await {
            let path = path.join("Path of Exile 2");

            info!("attempting to create game data directory at {path:?}");
            if fs::create_dir_all(&path)
                .await
                .inspect_err(|error| warn!("failed to create directory: {error:?}"))
                .is_ok()
            {
                info!("found game directory");
                return Ok(path);
            }
        }
    }

    Err(color_eyre::eyre::eyre!("No steam path could be located"))
}

fn releases_file(path: &Path) -> PathBuf {
    path.join("filter_watermarks.json")
}

fn to_cstr(os: &OsStr) -> CString {
    let mut bytes = os.as_bytes().to_vec();
    bytes.push(0);
    CString::from_vec_with_nul(bytes).unwrap()
}
