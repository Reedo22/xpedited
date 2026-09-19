use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::metadata::{ROLES, pick};

/// Steam keeps non-Steam shortcuts in a small binary form of VDF: a map is
/// 0x00 key NUL ... 0x08, a string is 0x01 key NUL value NUL, and an int is
/// 0x02 key NUL then four little endian bytes. Anything already in the file
/// belongs to the player, so it is parsed into this and written back out
/// unchanged rather than rebuilt from what we happen to know about.
#[derive(Debug, Clone)]
enum Vdf {
    Map(Vec<(String, Vdf)>),
    Str(String),
    Int(u32),
}

impl Vdf {
    fn get_str(&self, key: &str) -> Option<&str> {
        match self {
            Vdf::Map(entries) => {
                entries
                    .iter()
                    .find(|(k, _)| k == key)
                    .and_then(|(_, v)| match v {
                        Vdf::Str(s) => Some(s.as_str()),
                        _ => None,
                    })
            }
            _ => None,
        }
    }
}

fn parse_cstr(data: &[u8], at: &mut usize) -> Option<String> {
    let end = data[*at..].iter().position(|b| *b == 0)? + *at;
    let text = String::from_utf8_lossy(&data[*at..end]).into_owned();
    *at = end + 1;
    Some(text)
}

fn parse_map(data: &[u8], at: &mut usize) -> Option<Vdf> {
    let mut entries = vec![];
    loop {
        let marker = *data.get(*at)?;
        *at += 1;
        match marker {
            0x08 => return Some(Vdf::Map(entries)),
            0x00 => {
                let key = parse_cstr(data, at)?;
                entries.push((key, parse_map(data, at)?));
            }
            0x01 => {
                let key = parse_cstr(data, at)?;
                entries.push((key, Vdf::Str(parse_cstr(data, at)?)));
            }
            0x02 => {
                let key = parse_cstr(data, at)?;
                let bytes: [u8; 4] = data.get(*at..*at + 4)?.try_into().ok()?;
                *at += 4;
                entries.push((key, Vdf::Int(u32::from_le_bytes(bytes))));
            }
            _ => return None,
        }
    }
}

fn write_vdf(out: &mut Vec<u8>, value: &Vdf) {
    match value {
        Vdf::Map(entries) => {
            for (key, child) in entries {
                match child {
                    Vdf::Map(_) => {
                        out.push(0x00);
                        out.extend_from_slice(key.as_bytes());
                        out.push(0);
                        write_vdf(out, child);
                    }
                    Vdf::Str(text) => {
                        out.push(0x01);
                        out.extend_from_slice(key.as_bytes());
                        out.push(0);
                        out.extend_from_slice(text.as_bytes());
                        out.push(0);
                    }
                    Vdf::Int(number) => {
                        out.push(0x02);
                        out.extend_from_slice(key.as_bytes());
                        out.push(0);
                        out.extend_from_slice(&number.to_le_bytes());
                    }
                }
            }
            out.push(0x08);
        }
        _ => unreachable!("only maps are written at the top level"),
    }
}

/// Steam looks artwork up by the shortcut's own appid, so any stable value
/// with the high bit set will do as long as the files are named to match.
fn app_id(store_id: &str) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for byte in format!("xodus-{store_id}").bytes() {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    (!crc) | 0x8000_0000
}

/// Steam keeps the shortcut list in memory and writes it out when it exits,
/// so anything added underneath a running Steam is quietly thrown away. That
/// looks exactly like the export not working, so refuse instead.
fn steam_is_running() -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    entries.flatten().any(|entry| {
        std::fs::read_to_string(entry.path().join("comm")).is_ok_and(|comm| comm.trim() == "steam")
    })
}

fn steam_userdata(explicit: Option<String>) -> Option<PathBuf> {
    if let Some(explicit) = explicit {
        return Some(PathBuf::from(explicit));
    }
    let home = PathBuf::from(std::env::var("HOME").ok()?);
    for root in [
        home.join(".steam/steam"),
        home.join(".local/share/Steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ] {
        let Ok(users) = std::fs::read_dir(root.join("userdata")) else {
            continue;
        };
        for user in users.flatten() {
            if user.path().join("config").is_dir() {
                return Some(user.path());
            }
        }
    }
    None
}

async fn fetch_art(client: &reqwest::Client, url: &str, to: &Path) -> bool {
    if url.is_empty() || to.exists() {
        return false;
    }
    let Ok(response) = client.get(url).send().await else {
        return false;
    };
    let Ok(bytes) = response.bytes().await else {
        return false;
    };
    std::fs::File::create(to)
        .and_then(|mut file| file.write_all(&bytes))
        .is_ok()
}

/// A launcher beside the game, the same shape the Heroic export writes, so
/// Steam and Heroic can point at the same thing.
fn write_launcher(source: &Path, wine: &str, prefix: Option<&str>) -> std::io::Result<PathBuf> {
    let path = source.join("xodus-launch.sh");
    if path.exists() {
        return Ok(path);
    }
    let cli = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "xodus-cli".to_string());
    let prefix = prefix.unwrap_or("$HOME/.local/share/xodus/prefix");
    let script = format!(
        "#!/usr/bin/env bash\n\
         set -uo pipefail\n\
         XODUS_CLI=${{XODUS_CLI:-{cli}}}\n\
         XODUS_WINE=${{XODUS_WINE:-{wine}}}\n\
         export WINEPREFIX=${{WINEPREFIX:-{prefix}}}\n\
         exec \"$XODUS_CLI\" run '{source}' \"$XODUS_WINE\"\n",
        cli = cli,
        wine = wine,
        prefix = prefix,
        source = source.display(),
    );
    std::fs::write(&path, script)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    Ok(path)
}

pub async fn run(
    client: &reqwest::Client,
    wine: String,
    games_dirs: Vec<String>,
    market: Option<String>,
    prefix: Option<String>,
    userdata: Option<String>,
    force: bool,
    dry_run: bool,
) -> ExitCode {
    let market = market.unwrap_or("US".to_string());
    let home = std::env::var("HOME").unwrap_or_default();
    let games_dirs = if games_dirs.is_empty() {
        vec![format!("{home}/Games/xodus")]
    } else {
        games_dirs
    };

    if !dry_run && steam_is_running() && !force {
        eprintln!(
            "Steam is running. It holds the shortcut list in memory and writes it out\n\
             when it closes, so anything added now would be thrown away. Close Steam\n\
             and run this again, or pass --force if you know what you are doing."
        );
        return ExitCode::FAILURE;
    }

    let Some(userdata) = steam_userdata(userdata) else {
        eprintln!("could not find a Steam user directory; pass --userdata");
        return ExitCode::FAILURE;
    };
    let shortcuts_path = userdata.join("config").join("shortcuts.vdf");
    let grid = userdata.join("config").join("grid");

    // Find the extracted games.
    let mut games = vec![];
    for dir in &games_dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.join(".xodus-streaming.msixvc").exists() {
                continue;
            }
            let config = crate::gameconfig::read(&path);
            let store_id = config.as_deref().and_then(crate::gameconfig::store_id);
            games.push((path, store_id));
        }
    }
    if games.is_empty() {
        eprintln!("no extracted games found under {}", games_dirs.join(", "));
        return ExitCode::FAILURE;
    }

    let existing = std::fs::read(&shortcuts_path).unwrap_or_default();
    let mut root = if existing.is_empty() {
        Vdf::Map(vec![("shortcuts".to_string(), Vdf::Map(vec![]))])
    } else {
        let mut at = 0;
        match parse_map(&existing, &mut at) {
            Some(parsed) => parsed,
            None => {
                eprintln!(
                    "could not read {} - leaving it alone rather than risk your shortcuts",
                    shortcuts_path.display()
                );
                return ExitCode::FAILURE;
            }
        }
    };

    let Vdf::Map(top) = &mut root else {
        unreachable!()
    };
    let Some((_, Vdf::Map(shortcuts))) = top.iter_mut().find(|(key, _)| key == "shortcuts") else {
        eprintln!("{} has no shortcuts section", shortcuts_path.display());
        return ExitCode::FAILURE;
    };
    let kept = shortcuts.len();

    let mut added = 0usize;
    let mut arted = 0usize;
    for (path, store_id) in &games {
        // One lookup per game: the title, who made it, and the art all come
        // out of the same answer.
        let product = match store_id {
            Some(store_id) => xodus::api::displaycatalog::find_products_by_id(
                client,
                store_id.clone(),
                market.clone(),
                vec!["en-US".to_string()],
            )
            .await
            .ok()
            .map(|response| response.product),
            None => None,
        };
        let properties = product
            .as_ref()
            .and_then(|product| product.localized_properties.first());
        let title = properties
            .map(|props| props.product_title.clone())
            .filter(|title| !title.is_empty());
        let publisher = properties.map(|props| props.publisher_name.clone());
        let title = title.unwrap_or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Xbox game".to_string())
        });

        let launcher = if dry_run {
            path.join("xodus-launch.sh")
        } else {
            match write_launcher(path, &wine, prefix.as_deref()) {
                Ok(path) => path,
                Err(err) => {
                    eprintln!("could not write a launcher for {title}: {err}");
                    continue;
                }
            }
        };

        let mut tags = vec![("0".to_string(), Vdf::Str("Xbox".to_string()))];
        if let Some(publisher) = &publisher
            && !publisher.is_empty()
        {
            tags.push((tags.len().to_string(), Vdf::Str(publisher.clone())));
        }

        let id = app_id(store_id.as_deref().unwrap_or(&title));
        let exe = format!("\"{}\"", launcher.display());
        let entry = Vdf::Map(vec![
            ("appid".to_string(), Vdf::Int(id)),
            ("AppName".to_string(), Vdf::Str(title.clone())),
            ("Exe".to_string(), Vdf::Str(exe.clone())),
            (
                "StartDir".to_string(),
                Vdf::Str(format!("{}/", path.display())),
            ),
            ("icon".to_string(), Vdf::Str(String::new())),
            ("ShortcutPath".to_string(), Vdf::Str(String::new())),
            ("LaunchOptions".to_string(), Vdf::Str(String::new())),
            ("IsHidden".to_string(), Vdf::Int(0)),
            ("AllowDesktopConfig".to_string(), Vdf::Int(1)),
            ("AllowOverlay".to_string(), Vdf::Int(1)),
            ("OpenVR".to_string(), Vdf::Int(0)),
            ("Devkit".to_string(), Vdf::Int(0)),
            ("DevkitGameID".to_string(), Vdf::Str(String::new())),
            ("DevkitOverrideAppID".to_string(), Vdf::Int(0)),
            ("LastPlayTime".to_string(), Vdf::Int(0)),
            ("FlatpakAppID".to_string(), Vdf::Str(String::new())),
            // Steam turns these into library categories, which is the only
            // grouping a non-Steam shortcut gets. One for where the game came
            // from, and the publisher, which is worth having when several
            // hundred of these land in a library at once.
            ("tags".to_string(), Vdf::Map(tags)),
        ]);

        // Replace ours rather than pile up duplicates every run.
        match shortcuts
            .iter()
            .position(|(_, value)| value.get_str("Exe") == Some(exe.as_str()))
        {
            Some(index) => shortcuts[index].1 = entry,
            None => {
                let key = shortcuts.len().to_string();
                shortcuts.push((key, entry));
                added += 1;
            }
        }

        // Steam names shortcut artwork after the shortcut's own appid.
        if !dry_run && let Some(props) = properties {
            let _ = std::fs::create_dir_all(&grid);
            let art = |role: &str| {
                ROLES
                    .iter()
                    .find(|(name, _)| *name == role)
                    .and_then(|(_, purposes)| pick(&props.images, purposes))
                    .map(|image| image.absolute_uri())
                    .unwrap_or_default()
            };
            for (role, file) in [
                ("cover", format!("{id}p.jpg")),
                ("hero", format!("{id}_hero.jpg")),
                ("hero", format!("{id}.jpg")),
                ("logo", format!("{id}_logo.png")),
            ] {
                if fetch_art(client, &art(role), &grid.join(file)).await {
                    arted += 1;
                }
            }
        }
    }

    if dry_run {
        println!(
            "would add {added} games to Steam beside {kept} shortcuts already there, into {}",
            shortcuts_path.display()
        );
        return ExitCode::SUCCESS;
    }

    // Keep a copy of what was there. This file is the player's whole
    // non-Steam library and it is not ours to lose.
    if !existing.is_empty() {
        let _ = std::fs::write(shortcuts_path.with_extension("vdf.xodus-backup"), &existing);
    }

    let mut out = vec![];
    write_vdf(&mut out, &root);
    let temporary = shortcuts_path.with_extension("vdf.tmp");
    let written = std::fs::write(&temporary, &out)
        .and_then(|()| std::fs::rename(&temporary, &shortcuts_path));
    if let Err(err) = written {
        let _ = std::fs::remove_file(&temporary);
        eprintln!("could not write {}: {err}", shortcuts_path.display());
        return ExitCode::FAILURE;
    }

    println!("added {added} games to Steam, {kept} shortcuts already there were kept");
    println!(
        "  {arted} pieces of artwork fetched into {}",
        grid.display()
    );
    println!("  shortcuts  {}", shortcuts_path.display());
    println!("  a copy of the old file is beside it, ending .xodus-backup");
    println!("restart Steam to see them.");
    ExitCode::SUCCESS
}
