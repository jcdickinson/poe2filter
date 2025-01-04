use std::{
    ffi::OsString,
    io::{Cursor, Read as _},
    path::PathBuf,
};

use crate::{split, Globals, VersionInfo};
use color_eyre::{eyre::bail, Result};
use log::info;
use reqwest::header::HeaderValue;
use serde::Deserialize;
use tokio::{fs, io::AsyncWriteExt as _};

static API_VERSION: HeaderValue = HeaderValue::from_static("2022-11-28");
static API_JSON_TYPE: HeaderValue = HeaderValue::from_static("application/vnd.github+json");

#[derive(Debug, Clone, Deserialize)]
struct ReleaseInfo {
    zipball_url: String,
    tag_name: String,
    body: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct BranchInfo {
    commit: CommitInfo,
}

#[derive(Debug, Clone, Deserialize)]
struct CommitInfo {
    sha: String,
    commit: CommitMeta,
}

#[derive(Debug, Clone, Deserialize)]
struct CommitMeta {
    message: String,
}

pub async fn get(
    globals: &Globals,
    value: &str,
    existing: Option<&String>,
) -> Result<Option<VersionInfo>> {
    let parts = split(value, '/');
    let release = match parts.as_slice() {
        [owner, repo] => get_github_release(globals, owner, repo).await?,
        [owner, repo, branch] => get_github_branch(globals, owner, repo, branch).await?,
        _ => bail!("github source must be either github:owner/repo or github:owner/repo/branch"),
    };

    let Some(release) = release else {
        return Ok(None);
    };

    info!("found release with watermark: {}", release.watermark);

    if existing == Some(&release.watermark) {
        return Ok(None);
    }

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

    Ok(Some(release))
}

async fn get_github_branch(
    globals: &Globals,
    owner: &str,
    repo: &str,
    branch: &str,
) -> Result<Option<VersionInfo>> {
    info!("fetching latest commit");
    let release = globals
        .client
        .get(format!(
            "https://api.github.com/repos/{owner}/{repo}/branches/{branch}"
        ))
        .header("X-Github-Api-Version", API_VERSION.clone())
        .header("Accept", API_JSON_TYPE.clone())
        .send()
        .await?
        .error_for_status()?
        .json::<BranchInfo>()
        .await?;

    let zipball_url = format!(
        "https://github.com/{owner}/{repo}/archive/{}.zip",
        release.commit.sha
    );

    Ok(Some(VersionInfo {
        zipball_url,
        watermark: release.commit.sha,
        body: Some(release.commit.commit.message),
    }))
}

async fn get_github_release(
    globals: &Globals,
    owner: &str,
    repo: &str,
) -> Result<Option<VersionInfo>> {
    info!("fetching latest release");
    let releases = globals
        .client
        .get(format!(
            "https://api.github.com/repos/{owner}/{repo}/releases?per_page=1&page=0"
        ))
        .header("X-Github-Api-Version", API_VERSION.clone())
        .header("Accept", API_JSON_TYPE.clone())
        .send()
        .await?
        .error_for_status()?
        .json::<Vec<ReleaseInfo>>()
        .await?;

    let Some(release) = releases.into_iter().next() else {
        return Ok(None);
    };

    Ok(Some(VersionInfo {
        zipball_url: release.zipball_url,
        watermark: release.tag_name,
        body: release.body,
    }))
}
