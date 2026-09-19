use std::collections::HashMap;
use std::os::fd::{AsFd, IntoRawFd};
use std::path::Path;
use std::process::ExitCode;

use msixvc::layout::PAGE_SIZE;
use msixvc::xvd::{SegmentFile, XvdFile};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
#[cfg(target_os = "linux")]
use rustix::fs::{MemfdFlags, memfd_create};
use rustix::io::{FdFlags, fcntl_getfd, fcntl_setfd};
#[cfg(not(target_os = "linux"))]
use tempfile::{tempdir, tempfile, tempfile_in};
use tokio::fs::{File, OpenOptions};
use tokio::process::Command;
use xodus::tokens::TokenManager;

use crate::license::get_license;

#[cfg(target_os = "linux")]
fn make_temp_file(_folder: &str) -> std::io::Result<std::fs::File> {
    let fd = memfd_create("xodus", MemfdFlags::CLOEXEC).map_err(std::io::Error::from)?;
    Ok(std::fs::File::from(fd))
}

#[cfg(not(target_os = "linux"))]
fn make_temp_file(folder: &str) -> std::io::Result<std::fs::File> {
    if folder.is_empty() {
        tempfile()
    } else {
        tempfile_in(folder)
    }
}

#[cfg(target_os = "macos")]
async fn prepare(lfiles: &HashMap<String, SegmentFile>) -> (impl AsyncFnOnce(), String) {
    let disk_size: u64 = lfiles
        .iter()
        .filter(|f| f.1.keep_encrypted)
        .map(|f| f.1.length + 4 * PAGE_SIZE as u64)
        .reduce(|o, s| o + s)
        .unwrap();

    let device_s = String::from_utf8(
        Command::new("/usr/bin/hdiutil")
            .arg("attach")
            .arg("-nomount")
            .arg(format!("ram://{}", disk_size.div_ceil(256)))
            .output()
            .await
            .unwrap()
            .stdout,
    )
    .unwrap();

    let device = device_s.trim();

    let vol = uuid::Uuid::new_v4().to_string();

    let fmt = Command::new("/sbin/newfs_hfs")
        .arg("-v")
        .arg(vol)
        .arg(device)
        .status()
        .await
        .unwrap();
    assert!(fmt.success());

    let mount_dir_obj = tempdir().unwrap();
    let mount_dir = mount_dir_obj.path().to_str().unwrap();

    let mnt = Command::new("/sbin/mount")
        .arg("-t")
        .arg("hfs")
        .arg("-o")
        .arg("nobrowse")
        .arg("-v")
        .arg(device)
        .arg(mount_dir)
        .status()
        .await
        .unwrap();
    assert!(mnt.success());
    let mount_dir_cl = mount_dir.to_string();
    let device_cl = device.to_string();
    (
        async move || {
            let mnt = Command::new("/sbin/umount")
                .arg("-f")
                .arg(mount_dir_cl)
                .status()
                .await
                .unwrap();
            assert!(mnt.success());

            let mnt = Command::new("/usr/bin/hdiutil")
                .arg("detach")
                .arg("-force")
                .arg(&device_cl)
                .status()
                .await
                .unwrap();
            assert!(mnt.success());
        },
        mount_dir.to_owned(),
    )
}

#[cfg(not(target_os = "macos"))]
async fn prepare(_lfiles: &HashMap<String, SegmentFile>) -> (impl AsyncFnOnce(), String) {
    (async || {}, "".to_owned())
}

/// Ask Xbox Live who the signed in user is, so the runtime can tell the game.
/// A title that cannot be told who is playing will not start.
async fn resolve_identity(
    client: &reqwest::Client,
    tokens: &TokenManager,
) -> Option<(String, String)> {
    use xodus::models::secrets::Token;

    let Ok(Token::Legacy(dev_token)) = tokens.get_device_sts_token() else {
        return None;
    };
    let Ok(Token::Legacy(user_token)) = tokens.get_user_sts_token() else {
        return None;
    };

    let xsts = xodus::api::xbox::run(client, dev_token, user_token, "http://xboxlive.com").await;
    let xuid = xsts.xuid()?.to_string();
    let gamertag = xsts.gamertag().unwrap_or("").to_string();
    Some((xuid, gamertag))
}

pub async fn run(
    client: &reqwest::Client,
    tokens: &TokenManager,
    source: String,
    wine: String,
    exe: Option<String>,
    market: Option<String>,
) -> ExitCode {
    let mut lfiles: HashMap<String, SegmentFile> = HashMap::new();

    let out: &Path = Path::new(&source);
    let out_absolute = std::fs::canonicalize(out).unwrap();
    let final_path = out.join(".xodus-streaming.msixvc");

    let mut file = OpenOptions::new()
        .read(true)
        .open(final_path.to_owned())
        .await
        .unwrap();

    let xvd = XvdFile::parse(&mut file).await.expect("no err");

    let package_full_name = xvd.parse_package_full_name(&mut file).await.ok().flatten();

    let files = xvd.parse_user_package_files(&mut file).await.expect("ok");
    for (k, v) in &files {
        if k == "SegmentMetadata.bin" {
            let sfiles = xvd.parse_segment_metadata(&mut file, v).await.expect("ok");
            lfiles = sfiles;
        }
    }

    // Classic files
    if lfiles.is_empty() {
        let sfiles = xvd
            .parse_ntfs_segment_metadata(&mut file, !lfiles.is_empty())
            .await
            .expect("ok");
        for (n, sfile) in &sfiles {
            if sfile.length.div_ceil(PAGE_SIZE as u64) as usize != sfile.data_hashs.len() {
                println!("{}: {} {}", n, sfile.offset, sfile.length);
            }
        }
        lfiles.extend(sfiles);
    }

    let license = get_license(
        client,
        tokens,
        xvd.content_id().to_string(),
        market.unwrap_or("neutral".to_string()),
    )
    .await;
    if let Err(err) = license {
        eprintln!("{}", err);
        return ExitCode::FAILURE;
    }
    let (key, game_splicense) = license.unwrap();
    if game_splicense.content_keys.len() != 1 {
        eprintln!(
            "unexpected number of content keys {}",
            game_splicense.content_keys.len()
        );
        return ExitCode::FAILURE;
    }
    let Some((_, content_key)) = game_splicense.content_keys.into_iter().next() else {
        return ExitCode::FAILURE;
    };

    let full_key = content_key.unpack(&key).expect("failed to unpack");

    let mut fds = vec![];

    let (cleanup, mount_dir) = prepare(&lfiles).await;

    for file in &lfiles {
        if !file.1.keep_encrypted {
            continue;
        }
        let mut game_exe = File::from_std(make_temp_file(&mount_dir).unwrap());

        let source_path = out.join(file.0.replace("\\", "/"));

        let mut i = File::open(&source_path).await.unwrap();

        xvd.mount_mem_fd(&mut i, &mut game_exe, file.1, *full_key, |_, _| {})
            .await
            .unwrap();

        let stdf = game_exe.into_std().await;

        let mut flags = fcntl_getfd(stdf.as_fd()).unwrap();
        flags.remove(FdFlags::CLOEXEC);
        fcntl_setfd(stdf.as_fd(), flags).unwrap();

        fds.push((file.0, stdf.into_raw_fd()));
    }

    // The loader resolves each candidate with realpath() before comparing it
    // against WINE_EXE_FILE_MAP, so the entries have to be unix paths - an NT
    // path can never match, and the mapping silently does nothing.
    let mut env_value = String::new();

    let mut entry_path = None;

    for fd in fds {
        if !env_value.is_empty() {
            env_value.push('|');
        }

        let relative = fd.0.trim_start_matches('\\').replace('\\', "/");
        let unix_path = out_absolute.join(&relative).to_string_lossy().into_owned();
        if let Some(exe) = &exe {
            if exe == fd.0 {
                entry_path = Some(unix_path.clone())
            }
        } else if entry_path.is_none() {
            entry_path = Some(unix_path.clone())
        }

        env_value.push_str(&format!("{}:{}", fd.1, unix_path))
    }

    let Some(entry_path) = entry_path else {
        eprintln!("Could not find .exe");
        return ExitCode::FAILURE;
    };

    let mut command = Command::new(wine);
    command.arg(entry_path).env("WINE_EXE_FILE_MAP", env_value);

    // Wine's icu.dll is a forwarder to an icuuc68.dll that Wine does not
    // ship, so .NET's globalization cannot load and a managed title dies
    // before its first frame. Invariant mode costs culture-aware string
    // handling, which a game is unlikely to miss, and is the difference
    // between starting and not. Leave it alone if the caller set it.
    if std::env::var_os("DOTNET_SYSTEM_GLOBALIZATION_INVARIANT").is_none() {
        command.env("DOTNET_SYSTEM_GLOBALIZATION_INVARIANT", "1");
    }

    // A packaged title asks the runtime what package it is running as. The
    // container knows, and nothing inside the prefix does.
    if let Some(name) = &package_full_name {
        println!("package {name}");
        command.env("XODUS_PACKAGE_FULL_NAME", name);
    }

    match resolve_identity(client, tokens).await {
        Some((xuid, gamertag)) => {
            println!("signed in as {gamertag} ({xuid})");
            command.env("XODUS_USER_XUID", xuid);
            command.env("XODUS_USER_GAMERTAG", gamertag);
        }
        None => eprintln!("could not resolve the signed in user; the game will see nobody"),
    }

    let mut wn = command.spawn().unwrap();

    let pid = wn.id().unwrap();

    ctrlc::set_handler(move || {
        if pid > 0 {
            let _ = kill(Pid::from_raw(pid as i32), Signal::SIGINT);
        }
    })
    .expect("failed to install Ctrl+C handler");

    let status = wn.wait().await.unwrap();

    cleanup().await;

    ExitCode::from(status.code().map(|c| c as u8).unwrap_or(0))
}
