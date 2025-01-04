use std::{
    collections::{HashSet, VecDeque},
    env::{args_os, var_os},
    ffi::{CString, OsStr, OsString},
    io::{Cursor, Read},
    os::unix::prelude::OsStrExt,
    path::PathBuf,
};

use color_eyre::{eyre::eyre, Result};
use log::{debug, info, warn};
use reqwest::{
    header::{HeaderMap, HeaderValue},
    ClientBuilder,
};
use serde::Deserialize;
use tokio::fs;

#[derive(Debug, Clone, Deserialize)]
struct ReleaseInfo {
    zipball_url: String,
    tag_name: String,
    body: Option<String>,
}

fn main() -> Result<()> {
    pretty_env_logger::init_custom_env("POE2FILTER_LOG");

    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("spawn async runtime");

        rt.block_on(async_main())?;
    }

    // Future-proof, do nothing if there isn't a chained exec separator

    let sep = OsString::from("--");
    let mut args: VecDeque<_> = args_os().collect();

    debug!("args are {args:?}");

    while let Some(front) = args.pop_front() {
        if front == sep {
            break;
        }
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

fn to_cstr(os: &OsStr) -> CString {
    let mut bytes = os.as_bytes().to_vec();
    bytes.push(0);
    CString::from_vec_with_nul(bytes).unwrap()
}

async fn async_main() -> color_eyre::Result<()> {
    info!("finding game directory");
    let dir = locate_game_directory().await?;

    let mut headers = HeaderMap::new();
    headers.insert(
        "X-GitHub-Api-Version",
        HeaderValue::from_static("2022-11-28"),
    );

    let client = ClientBuilder::new()
        .default_headers(headers)
        .user_agent("poe2filter")
        .build()?;

    info!("fetching latest release");
    let releases = client.get("https://api.github.com/repos/NeverSinkDev/NeverSink-PoE2litefilter/releases?per_page=1&page=0")
        .header("Accept",  HeaderValue::from_static("application/vnd.github+json"))
        .send()
        .await?
        .json::<Vec<ReleaseInfo>>()
        .await?;

    let release = releases
        .first()
        .ok_or_else(|| eyre!("no release could be found"))?;

    info!("found release with tag: {}", release.tag_name);

    let current_file = dir.join("installed_tag");
    if let Ok(installed_tag) = fs::read_to_string(&current_file).await {
        if installed_tag == release.tag_name {
            info!("filter is up to date, nothing to do");
            return Ok(());
        }
    }

    if let Some(body) = release.body.as_ref() {
        eprintln!("{body}");
    }

    info!("downloading release zipball");
    let zipball = client
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

        info!("extracting ${filename}");
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

        let full_path = dir.join(&filename);

        info!("writing ${full_path:?}");
        let mut dest = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(full_path)
            .await?;

        tokio::io::copy(&mut Cursor::new(&file_data), &mut dest).await?;
    }

    info!("writing update info to {current_file:?}...");
    let mut dest = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(current_file)
        .await?;
    tokio::io::copy(&mut Cursor::new(release.tag_name.as_bytes()), &mut dest).await?;

    info!("update complete");

    Ok(())
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
