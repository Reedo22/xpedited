//! What the app remembers between runs, so the window can be opened with
//! no arguments. The command line still wins when it says something.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_games_dir")]
    pub games_dir: String,
    #[serde(default = "default_wine")]
    pub wine: String,
    #[serde(default = "default_prefix")]
    pub prefix: String,
    #[serde(default = "default_market")]
    pub market: String,
    /// Where "report this" sends you. owner/name on GitHub.
    #[serde(default = "default_repo")]
    pub report_repo: String,
}

fn default_repo() -> String {
    "Reedo22/xpedited".to_string()
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

fn default_games_dir() -> String {
    format!("{}/Games/xodus", home())
}

fn default_prefix() -> String {
    format!("{}/.local/share/xpedited/prefix", home())
}

fn default_market() -> String {
    "US".to_string()
}

/// A guess for the first run; overwritten once `--wine` is given.
///
/// Games need the patched build, so a distribution Wine on PATH is only a
/// starting point - it will not run most titles.
fn default_wine() -> String {
    if let Ok(from_env) = std::env::var("XPEDITED_WINE")
        && std::path::Path::new(&from_env).is_file()
    {
        return from_env;
    }
    let home = home();
    for candidate in [
        format!("{home}/.local/share/xpedited/wine/bin/wine"),
        "/usr/local/bin/wine".to_string(),
        "/usr/bin/wine".to_string(),
    ] {
        if std::path::Path::new(&candidate).is_file() {
            return candidate;
        }
    }
    "wine".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            games_dir: default_games_dir(),
            wine: default_wine(),
            prefix: default_prefix(),
            market: default_market(),
            report_repo: default_repo(),
        }
    }
}

pub fn path() -> PathBuf {
    PathBuf::from(format!("{}/.config/xpedited/settings.json", home()))
}

pub fn load() -> Settings {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(settings: &Settings) -> std::io::Result<()> {
    let path = path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(settings)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    // Write beside it and rename, so a crash mid-write cannot leave the
    // player with a settings file that will not parse.
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, &path)
}
