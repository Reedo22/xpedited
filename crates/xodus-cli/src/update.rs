//! Checking for, and installing, a newer build from GitHub releases.
//!
//! Nothing downloads until asked, the download must match the size the
//! release advertises, and the binary is replaced only once it is on disk.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    size: u64,
    browser_download_url: String,
}

pub struct Available {
    pub version: String,
    pub notes: String,
    url: String,
    size: u64,
}

/// Compare two dotted versions, ignoring a leading `v`.
fn newer(candidate: &str, current: &str) -> bool {
    let parts = |text: &str| {
        text.trim_start_matches(['v', 'V'])
            .split(['.', '-', '+'])
            .map(|piece| piece.parse::<u32>().unwrap_or(0))
            .take(3)
            .collect::<Vec<_>>()
    };
    let (a, b) = (parts(candidate), parts(current));
    for index in 0..3 {
        let (left, right) = (a.get(index).copied(), b.get(index).copied());
        match (left.unwrap_or(0), right.unwrap_or(0)) {
            (l, r) if l > r => return true,
            (l, r) if l < r => return false,
            _ => {}
        }
    }
    false
}

/// Which asset to download: the one built for this platform.
fn wanted_asset(assets: &[Asset]) -> Option<&Asset> {
    let arch = std::env::consts::ARCH;
    assets
        .iter()
        .find(|asset| asset.name.contains(arch) && !asset.name.ends_with(".sha256"))
        .or_else(|| assets.iter().find(|asset| asset.name.contains("linux")))
}

pub async fn check(
    client: &reqwest::Client,
    repo: &str,
) -> Result<Option<Available>, Box<dyn std::error::Error + Send + Sync>> {
    if !crate::crashreport::valid_repo(repo) {
        return Err("that is not an owner/name repository".into());
    }
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let response = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None); // no releases published yet
    }
    let release: Release = response.error_for_status()?.json().await?;

    if !newer(&release.tag_name, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let Some(asset) = wanted_asset(&release.assets) else {
        return Err("that release has no build for this machine".into());
    };
    Ok(Some(Available {
        version: release.tag_name,
        notes: release.body,
        url: asset.browser_download_url.clone(),
        size: asset.size,
    }))
}

/// Download the new build and put it where this one is running from.
pub async fn install(
    client: &reqwest::Client,
    update: &Available,
) -> Result<std::path::PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    use std::os::unix::fs::PermissionsExt;

    let target = std::env::current_exe()?;
    let bytes = client
        .get(&update.url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    if bytes.len() as u64 != update.size {
        return Err("the download did not match the size the release advertised".into());
    }

    // Write beside the running binary, then rename over it. A rename is
    // atomic, so a failure part way through cannot leave a half-written
    // program in place of a working one.
    let beside = target.with_extension("new");
    std::fs::write(&beside, &bytes)?;
    std::fs::set_permissions(&beside, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(&beside, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_sensibly() {
        assert!(newer("0.2.0", "0.1.0"));
        assert!(newer("v1.0.0", "0.9.9"));
        assert!(newer("0.1.1", "0.1.0"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.2.0"));
        assert!(!newer("v0.1.0", "0.1.0"));
        // Nonsense must not read as an upgrade.
        assert!(!newer("", "0.1.0"));
        assert!(!newer("banana", "0.1.0"));
    }

    #[test]
    fn the_asset_for_this_machine_is_chosen() {
        let assets = vec![
            Asset { name: "xpedited-aarch64".into(), size: 1, browser_download_url: "a".into() },
            Asset { name: "xpedited-x86_64".into(), size: 2, browser_download_url: "b".into() },
            Asset { name: "xpedited-x86_64.sha256".into(), size: 3, browser_download_url: "c".into() },
        ];
        let chosen = wanted_asset(&assets).expect("an asset");
        assert!(chosen.name.contains(std::env::consts::ARCH));
        assert!(!chosen.name.ends_with(".sha256"));
    }

    #[test]
    fn no_assets_means_no_choice() {
        assert!(wanted_asset(&[]).is_none());
    }
}
