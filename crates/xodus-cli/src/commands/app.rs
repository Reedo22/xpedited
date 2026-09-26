//! A window over everything the other commands do: browse the catalogue,
//! install, play. The interface is a webview because tao and wry are already
//! here for the sign in flow, and because a grid of box art is a thing HTML
//! is good at.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc;

use serde::{Deserialize, Serialize};
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

use super::metadata::{ROLES, pick};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Game {
    id: String,
    title: String,
    #[serde(default)]
    publisher: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    cover: String,
    #[serde(default)]
    hero: String,
    #[serde(default)]
    logo: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    installed: bool,
    /// "gamepass" or "owned" - a title can be both, and owned wins.
    #[serde(default)]
    source: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Request {
    Ready,
    Install { id: String },
    Play { id: String },
    SignIn,
    SignOut,
    Refresh,
    Stop { id: String },
    Uninstall { id: String },
    Report { id: String },
    CheckUpdate,
    OpenLog { id: String },
    SaveSettings {
        games_dir: String,
        wine: String,
        prefix: String,
        market: String,
        report_repo: String,
    },
}

/// What the window is told, from whichever thread has something to say.
enum Message {
    Catalogue(Vec<Game>),
    /// Who is signed in, for the account block the Xbox app leads with.
    Account {
        gamertag: String,
        signed_in: bool,
    },
    /// The current settings, so the settings page can show real values
    /// rather than the placeholders it was built with.
    Settings {
        games_dir: String,
        wine: String,
        prefix: String,
        market: String,
        report_repo: String,
        catalogue: usize,
        refreshed: String,
    },
    /// A one-line result for something the player asked for.
    Status(String),
    /// A game ended badly, and there is a log about it.
    Crashed {
        id: String,
        title: String,
        reason: String,
    },
    /// A game started or stopped, so the window can offer the other one.
    Running {
        id: String,
        running: bool,
    },
    /// Everything that was going to load has loaded. Until this arrives the
    /// window says it is still working rather than "you have no games".
    Ready,
    Progress {
        id: String,
        percent: u8,
        /// Bytes on disk so far, and what we are expecting in total. The
        /// window formats these, so it can say "142 MB of 2.2 GB" rather
        /// than rounding a small game away to nothing.
        have: u64,
        total: u64,
        /// Bytes per second over the last sample, or zero if not known yet.
        rate: u64,
        note: String,
    },
    Installed {
        id: String,
    },
}

/// The titles on the account itself, which the Game Pass catalogue knows
/// nothing about - Minecraft, and anything else bought outright.
async fn load_owned(
    client: &reqwest::Client,
    tokens: &xodus::tokens::TokenManager,
    market: &str,
    games_dir: &str,
) -> Vec<Game> {
    let Ok((device_token, user_token, puid)) = super::library::ms_tokens(client, tokens).await
    else {
        return vec![];
    };
    let Ok(collection) = xodus::api::collections::query_collection(
        client,
        device_token,
        user_token,
        puid,
        market.to_string(),
    )
    .await
    else {
        return vec![];
    };

    let ids: Vec<String> = collection
        .items
        .iter()
        .filter(|item| item.is_game())
        .map(|item| item.product_id.clone())
        .collect();

    let languages = vec!["en-US".to_string()];
    let mut games = vec![];
    for chunk in ids.chunks(12) {
        let Ok(response) =
            xodus::api::displaycatalog::find_products_by_ids(client, chunk, market, &languages)
                .await
        else {
            continue;
        };
        for product in response.products {
            if let Some(game) = game_from(&product, games_dir, "owned") {
                games.push(game);
            }
        }
    }
    games
}

/// Everything the interface needs about one product.
fn game_from(
    product: &xodus::models::displaycatalog::Product,
    games_dir: &str,
    source: &str,
) -> Option<Game> {
    let props = product.localized_properties.first()?;
    let art = |role: &str| {
        ROLES
            .iter()
            .find(|(name, _)| *name == role)
            .and_then(|(_, purposes)| pick(&props.images, purposes))
            .map(|image| image.absolute_uri())
            .unwrap_or_default()
    };
    let size = product
        .display_sku_availabilities
        .iter()
        .flat_map(|sku| sku.sku.properties.packages.iter())
        .filter(|package| {
            package
                .platform_dependencies
                .iter()
                .any(|dep| dep.platform_name == "Windows.Desktop")
        })
        .map(|package| package.max_download_size_in_bytes)
        .max()
        .unwrap_or(0);

    Some(Game {
        installed: installed_at(games_dir, &product.product_id),
        id: product.product_id.clone(),
        title: props.product_title.clone(),
        publisher: props.publisher_name.clone(),
        description: if props.short_description.is_empty() {
            props.product_description.clone()
        } else {
            props.short_description.clone()
        },
        cover: art("cover"),
        hero: art("hero"),
        logo: art("logo"),
        size,
        source: source.to_string(),
    })
}

/// When the cached catalogue was last written, in the plainest terms that
/// are still true.
fn catalogue_age() -> String {
    let Ok(modified) = std::fs::metadata(cache_path()).and_then(|meta| meta.modified()) else {
        return "never".to_string();
    };
    let Ok(elapsed) = modified.elapsed() else {
        return "just now".to_string();
    };
    let minutes = elapsed.as_secs() / 60;
    match minutes {
        0 => "just now".to_string(),
        1 => "a minute ago".to_string(),
        2..=59 => format!("{minutes} minutes ago"),
        60..=119 => "an hour ago".to_string(),
        120..=1439 => format!("{} hours ago", minutes / 60),
        1440..=2879 => "yesterday".to_string(),
        _ => format!("{} days ago", minutes / 1440),
    }
}

fn cache_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(format!("{home}/.cache/xpedited/catalogue.json"))
}

fn installed_at(games_dir: &str, id: &str) -> bool {
    PathBuf::from(games_dir)
        .join(id)
        .join(".xodus-streaming.msixvc")
        .exists()
}

/// The catalogue is slow to gather and changes rarely, so it is kept on disk
/// and only rebuilt when it is missing or the caller asks.
async fn load_catalogue(
    client: &reqwest::Client,
    market: &str,
    games_dir: &str,
    refresh: bool,
) -> Vec<Game> {
    let path = cache_path();
    if !refresh
        && let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(mut games) = serde_json::from_str::<Vec<Game>>(&text)
    {
        for game in &mut games {
            game.installed = installed_at(games_dir, &game.id);
        }
        return games;
    }

    let Ok(ids) = xodus::api::gamepass::pc_catalog(client, market).await else {
        return vec![];
    };
    let languages = vec!["en-US".to_string()];
    let mut games = vec![];
    for chunk in ids.chunks(12) {
        let Ok(response) =
            xodus::api::displaycatalog::find_products_by_ids(client, chunk, market, &languages)
                .await
        else {
            continue;
        };
        for product in response.products {
            if let Some(game) = game_from(&product, games_dir, "gamepass") {
                games.push(game);
            }
        }
    }
    games.sort_by_key(|game| game.title.to_lowercase());

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string(&games) {
        let _ = std::fs::write(&path, text);
    }
    games
}

fn directory_size(path: &PathBuf) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => directory_size(&entry.path()),
            Ok(_) => entry.metadata().map(|meta| meta.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

/// Somewhere to put everything a child process says, and the path to it.
fn open_log(id: &str, kind: &str) -> (PathBuf, std::process::Stdio, std::process::Stdio) {
    let _ = std::fs::create_dir_all(crate::crashreport::log_dir());
    crate::crashreport::prune(20);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    let path = crate::crashreport::log_dir().join(format!("{id}-{kind}-{stamp}.log"));
    match std::fs::File::create(&path) {
        Ok(file) => match (file.try_clone(), file.try_clone()) {
            (Ok(out), Ok(err)) => (path, out.into(), err.into()),
            _ => (path, std::process::Stdio::null(), std::process::Stdio::null()),
        },
        Err(_) => (path, std::process::Stdio::null(), std::process::Stdio::null()),
    }
}

/// Whether a process's argv[0] names a program inside `dir`.
///
/// Wine gives the game its Windows path, so the same executable appears as
/// either `/home/you/Games/x/Game.exe` or `Z:\\home\\you\\Games\\x\\Game.exe`.
/// Both have to match, or stopping a game silently does nothing.
fn runs_from(argv0: &str, dir: &str) -> bool {
    let unix = argv0.replace('\\', "/");
    let unix = match unix.as_bytes() {
        [drive, b':', ..] if drive.is_ascii_alphabetic() => &unix[2..],
        _ => unix.as_str(),
    };
    unix.starts_with(&format!("{dir}/"))
}

/// Every process running out of a game's folder.
///
/// Wine reparents a game away from whatever launched it, so neither the
/// child handle nor its process group finds it again. Its argv[0] does.
fn game_processes(dir: &str) -> Vec<i32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return vec![];
    };
    let mut found = vec![];
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        let Ok(raw) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        // argv[0] only. Matching any argument would also catch the
        // launcher, an in-progress download, and this very process.
        let Some(argv0) = raw.split(|byte| *byte == 0).next() else {
            continue;
        };
        if runs_from(&String::from_utf8_lossy(argv0), dir) {
            found.push(pid);
        }
    }
    found
}

/// Ask a game to close, and insist if it will not.
fn stop_game(dir: String) {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;

    std::thread::spawn(move || {
        for pid in game_processes(&dir) {
            let _ = kill(Pid::from_raw(pid), Signal::SIGTERM);
        }
        std::thread::sleep(std::time::Duration::from_secs(8));
        for pid in game_processes(&dir) {
            let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
        }
    });
}

/// Store ids are alphanumeric. Anything else is not safe to hand to a
/// recursive delete.
fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() < 64 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Held for the life of the window. A second copy would fight this one over
/// the same downloads and settings, so it bows out instead.
fn claim_single_instance() -> Option<std::fs::File> {
    use rustix::fs::{FlockOperation, flock};

    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(format!("{dir}/xpedited.lock"))
        .ok()?;
    // Advisory, and released by the kernel however the process ends, so a
    // crash cannot leave the app permanently unable to start.
    flock(&file, FlockOperation::NonBlockingLockExclusive).ok()?;
    Some(file)
}

pub async fn run(
    tokens: &xodus::tokens::TokenManager,
    wine: Option<String>,
    games_dir: Option<String>,
    market: Option<String>,
    prefix: Option<String>,
    refresh: bool,
) -> ExitCode {
    let Some(_instance) = claim_single_instance() else {
        println!("Xpedited is already open.");
        return ExitCode::SUCCESS;
    };

    // Anything the command line says is both used now and remembered, so the
    // window can be opened from a desktop entry with no arguments next time.
    let mut stored = crate::settings::load();
    let mut changed = false;
    for (value, slot) in [
        (wine, &mut stored.wine),
        (games_dir, &mut stored.games_dir),
        (market, &mut stored.market),
        (prefix, &mut stored.prefix),
    ] {
        if let Some(value) = value
            && *slot != value
        {
            *slot = value;
            changed = true;
        }
    }
    // Write on the first run too, so a desktop entry needs no arguments.
    if (changed || !crate::settings::path().exists())
        && let Err(err) = crate::settings::save(&stored)
    {
        eprintln!("could not save settings: {err}");
    }
    let crate::settings::Settings {
        games_dir,
        wine,
        prefix,
        market,
        mut report_repo,
    } = stored;
    let cli = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "xodus-cli".to_string());

    let event_loop = EventLoopBuilder::<Message>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = match WindowBuilder::new()
        .with_title("Xpedited")
        .with_inner_size(LogicalSize::new(1320.0, 840.0))
        .build(&event_loop)
    {
        Ok(window) => window,
        Err(err) => {
            eprintln!("could not open a window: {err}");
            return ExitCode::FAILURE;
        }
    };

    let (requests, incoming) = mpsc::channel::<Request>();
    let builder = WebViewBuilder::new()
        .with_html(include_str!("app.html"))
        .with_ipc_handler(move |request| {
            if let Ok(parsed) = serde_json::from_str::<Request>(request.body()) {
                let _ = requests.send(parsed);
            }
        });

    // On this platform the webview lives in the window's GTK container
    // rather than being handed the window itself.
    #[cfg(target_os = "linux")]
    let built = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(window.default_vbox().expect("a gtk container"))
    };
    #[cfg(not(target_os = "linux"))]
    let built = builder.build(&window);

    let webview = match built {
        Ok(webview) => webview,
        Err(err) => {
            eprintln!("could not start the interface: {err}");
            return ExitCode::FAILURE;
        }
    };

    // Everything that talks to the network or spawns a download runs off the
    // window's thread, and reports back through the event loop.
    {
        let proxy = proxy.clone();
        let mut games_dir = games_dir.clone();
        let mut market = market.clone();
        let mut wine = wine.clone();
        let mut prefix = prefix.clone();
        let cli = cli.clone();
        let tokens_for_account = tokens.clone();
        // id -> the folder its processes are running out of.
        let running: std::sync::Arc<std::sync::Mutex<HashMap<String, String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(HashMap::new()));
        // Games the player closed on purpose, which are not failures.
        let stopped: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
        // id -> (why it failed, where its log is).
        let failures: std::sync::Arc<std::sync::Mutex<HashMap<String, (String, PathBuf)>>> =
            std::sync::Arc::new(std::sync::Mutex::new(HashMap::new()));
        std::thread::spawn(move || {
            // This thread makes a handful of requests one after another, so
            // it has no use for a worker thread per core.
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime");
            let client = reqwest::Client::builder()
                .user_agent(concat!("xpedited/", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("a client");

            // From disk, so the window fills at once; network work follows.
            let mut games = runtime.block_on(load_catalogue(&client, &market, &games_dir, refresh));
            let _ = proxy.send_event(Message::Catalogue(games.clone()));

            let announce_account = |identity: Option<(String, String)>| match identity {
                Some((_, gamertag)) if !gamertag.is_empty() => Message::Account {
                    gamertag,
                    signed_in: true,
                },
                Some(_) => Message::Account {
                    gamertag: "Signed in".to_string(),
                    signed_in: true,
                },
                None => Message::Account {
                    gamertag: "Sign in".to_string(),
                    signed_in: false,
                },
            };
            let identity =
                runtime.block_on(super::run::resolve_identity(&client, &tokens_for_account));
            let mut private = crate::crashreport::Private::here(identity.as_ref());
            let _ = proxy.send_event(announce_account(identity));

            // Owned titles take precedence: a game can be both, and owning
            // it is the more useful thing to be told.
            let owned = runtime.block_on(load_owned(
                &client,
                &tokens_for_account,
                &market,
                &games_dir,
            ));
            if !owned.is_empty() {
                for game in owned {
                    match games.iter_mut().find(|other| other.id == game.id) {
                        Some(existing) => existing.source = "owned".to_string(),
                        None => games.push(game),
                    }
                }
                games.sort_by_key(|game| game.title.to_lowercase());
                let _ = proxy.send_event(Message::Catalogue(games.clone()));
            }
            let _ = proxy.send_event(Message::Ready);

            let describe = |games_dir: &str,
                            wine: &str,
                            prefix: &str,
                            market: &str,
                            report_repo: &str,
                            count| Message::Settings {
                games_dir: games_dir.to_string(),
                wine: wine.to_string(),
                prefix: prefix.to_string(),
                market: market.to_string(),
                report_repo: report_repo.to_string(),
                catalogue: count,
                refreshed: catalogue_age(),
            };
            let _ = proxy.send_event(describe(
                &games_dir,
                &wine,
                &prefix,
                &market,
                &report_repo,
                games.len(),
            ));

            while let Ok(request) = incoming.recv() {
                match request {
                    Request::Ready => {
                        for game in &mut games {
                            game.installed = installed_at(&games_dir, &game.id);
                        }
                        let _ = proxy.send_event(Message::Catalogue(games.clone()));
                    }
                    Request::SignIn => {
                        // The login flow needs a webview of its own, so it
                        // runs as a child process rather than in this window.
                        let status = std::process::Command::new(&cli).arg("login").status();
                        let identity = match status {
                            Ok(code) if code.success() => runtime
                                .block_on(super::run::resolve_identity(&client, &tokens_for_account)),
                            _ => None,
                        };
                        let signed_in = identity.is_some();
                        private = crate::crashreport::Private::here(identity.as_ref());
                        let _ = proxy.send_event(announce_account(identity));
                        let _ = proxy.send_event(Message::Status(
                            if signed_in { "Signed in." } else { "Sign in did not finish." }
                                .to_string(),
                        ));
                    }
                    Request::SignOut => {
                        let status = std::process::Command::new(&cli).arg("logout").status();
                        let _ = proxy.send_event(announce_account(None));
                        let _ = proxy.send_event(Message::Status(
                            match status {
                                Ok(code) if code.success() => "Signed out.",
                                _ => "Sign out reported a problem; the tokens may still be there.",
                            }
                            .to_string(),
                        ));
                    }
                    Request::Refresh => {
                        let _ = proxy.send_event(Message::Status(
                            "Rebuilding the catalogue…".to_string(),
                        ));
                        let rebuilt =
                            runtime.block_on(load_catalogue(&client, &market, &games_dir, true));
                        if rebuilt.is_empty() {
                            let _ = proxy.send_event(Message::Status(
                                "The catalogue came back empty; keeping the old one.".to_string(),
                            ));
                        } else {
                            // Owned titles are not in the Game Pass catalogue,
                            // so carry them across rather than losing them.
                            let mine: Vec<Game> = games
                                .iter()
                                .filter(|game| game.source == "owned")
                                .cloned()
                                .collect();
                            games = rebuilt;
                            for game in mine {
                                match games.iter_mut().find(|other| other.id == game.id) {
                                    Some(existing) => existing.source = "owned".to_string(),
                                    None => games.push(game),
                                }
                            }
                            games.sort_by_key(|game| game.title.to_lowercase());
                            let _ = proxy.send_event(Message::Catalogue(games.clone()));
                            let _ = proxy.send_event(Message::Status(format!(
                                "{} games in the catalogue.",
                                games.len()
                            )));
                        }
                        let _ = proxy.send_event(describe(
                            &games_dir,
                            &wine,
                            &prefix,
                            &market,
                            &report_repo,
                            games.len(),
                        ));
                    }
                    Request::SaveSettings {
                        games_dir: new_games_dir,
                        wine: new_wine,
                        prefix: new_prefix,
                        market: new_market,
                        report_repo: new_repo,
                    } => {
                        let market_changed = new_market != market;
                        games_dir = new_games_dir;
                        wine = new_wine;
                        prefix = new_prefix;
                        market = new_market;
                        report_repo = new_repo;
                        let saved = crate::settings::save(&crate::settings::Settings {
                            games_dir: games_dir.clone(),
                            wine: wine.clone(),
                            prefix: prefix.clone(),
                            market: market.clone(),
                            report_repo: report_repo.clone(),
                        });
                        // Which games look installed depends on the folder.
                        for game in &mut games {
                            game.installed = installed_at(&games_dir, &game.id);
                        }
                        let _ = proxy.send_event(Message::Catalogue(games.clone()));
                        let _ = proxy.send_event(Message::Status(match saved {
                            Ok(()) if market_changed => {
                                "Saved. Refresh the catalogue to use the new region.".to_string()
                            }
                            Ok(()) => "Saved.".to_string(),
                            Err(err) => format!("Could not save: {err}"),
                        }));
                        let _ = proxy.send_event(describe(
                            &games_dir,
                            &wine,
                            &prefix,
                            &market,
                            &report_repo,
                            games.len(),
                        ));
                    }
                    Request::Play { id } => {
                        let dir = format!("{games_dir}/{id}");
                        let title = games
                            .iter()
                            .find(|game| game.id == id)
                            .map(|game| game.title.clone())
                            .unwrap_or_else(|| id.clone());
                        // A game that will not start should say so rather
                        // than leaving the player clicking at nothing.
                        if !std::path::Path::new(&wine).is_file() {
                            let _ = proxy.send_event(Message::Status(format!(
                                "Cannot start {title}: no Wine at {wine}. Set it in Settings."
                            )));
                        } else if !std::path::Path::new(&dir).is_dir() {
                            let _ = proxy.send_event(Message::Status(format!(
                                "Cannot start {title}: {dir} is not there any more."
                            )));
                        } else {
                            // Keep what the game and Wine say.
                            let (log_path, out, err) = open_log(&id, "play");

                            match std::process::Command::new(&cli)
                                .args(["run", &dir, &wine])
                                .env("WINEPREFIX", &prefix)
                                .stdout(out)
                                .stderr(err)
                                .spawn()
                            {
                                Err(err) => {
                                    let _ = proxy.send_event(Message::Status(format!(
                                        "Cannot start {title}: {err}"
                                    )));
                                }
                                Ok(mut child) => {
                                    running.lock().unwrap().insert(id.clone(), dir.clone());
                                    let _ = proxy.send_event(Message::Running {
                                        id: id.clone(),
                                        running: true,
                                    });
                                    let _ = proxy
                                        .send_event(Message::Status(format!("Starting {title}…")));

                                    // The launcher can exit while the game
                                    // runs on, so wait for the folder to go
                                    // quiet instead.
                                    let proxy = proxy.clone();
                                    let running = running.clone();
                                    let stopped = stopped.clone();
                                    let failures = failures.clone();
                                    let started = std::time::Instant::now();
                                    std::thread::spawn(move || {
                                        let mut status = None;
                                        loop {
                                            std::thread::sleep(
                                                std::time::Duration::from_secs(3),
                                            );
                                            if status.is_none()
                                                && let Ok(Some(finished)) = child.try_wait()
                                            {
                                                status = Some(finished);
                                            }
                                            if status.is_some()
                                                && game_processes(&dir).is_empty()
                                            {
                                                break;
                                            }
                                        }
                                        running.lock().unwrap().remove(&id);
                                        let _ = proxy.send_event(Message::Running {
                                            id: id.clone(),
                                            running: false,
                                        });

                                        // Closing it yourself is not a failure.
                                        let by_hand =
                                            stopped.lock().unwrap().remove(&id);
                                        let log = std::fs::read_to_string(&log_path)
                                            .unwrap_or_default();
                                        let reason = if by_hand {
                                            None
                                        } else {
                                            crate::crashreport::diagnose(
                                                status.and_then(|s| s.code()),
                                                &log,
                                                started.elapsed().as_secs(),
                                            )
                                        };
                                        match reason {
                                            Some(reason) => {
                                                failures.lock().unwrap().insert(
                                                    id.clone(),
                                                    (reason.clone(), log_path.clone()),
                                                );
                                                let _ = proxy.send_event(Message::Crashed {
                                                    id,
                                                    title,
                                                    reason,
                                                });
                                            }
                                            None => {
                                                let _ = proxy.send_event(Message::Status(
                                                    format!("{title} closed."),
                                                ));
                                            }
                                        }
                                    });
                                }
                            }
                        }
                    }
                    Request::Stop { id } => {
                        let title = games
                            .iter()
                            .find(|game| game.id == id)
                            .map(|game| game.title.clone())
                            .unwrap_or_else(|| id.clone());
                        let folder = running.lock().unwrap().get(&id).cloned();
                        match folder {
                            Some(folder) => {
                                stopped.lock().unwrap().insert(id.clone());
                                stop_game(folder);
                                let _ = proxy
                                    .send_event(Message::Status(format!("Closing {title}…")));
                            }
                            None => {
                                let _ = proxy.send_event(Message::Status(format!(
                                    "{title} is not running."
                                )));
                            }
                        }
                    }
                    Request::Uninstall { id } => {
                        let title = games
                            .iter()
                            .find(|game| game.id == id)
                            .map(|game| game.title.clone())
                            .unwrap_or_else(|| id.clone());
                        let dir = PathBuf::from(format!("{games_dir}/{id}"));

                        if !safe_id(&id) {
                            let _ = proxy.send_event(Message::Status(
                                "That does not look like a game folder; leaving it alone."
                                    .to_string(),
                            ));
                        } else if running.lock().unwrap().contains_key(&id) {
                            let _ = proxy.send_event(Message::Status(format!(
                                "{title} is running. Close it first."
                            )));
                        } else if !dir.is_dir() {
                            let _ = proxy.send_event(Message::Status(format!(
                                "{title} is not installed."
                            )));
                        } else {
                            let freed = directory_size(&dir);
                            match std::fs::remove_dir_all(&dir) {
                                Ok(()) => {
                                    if let Some(game) =
                                        games.iter_mut().find(|game| game.id == id)
                                    {
                                        game.installed = false;
                                    }
                                    let _ = proxy.send_event(Message::Catalogue(games.clone()));
                                    let _ = proxy.send_event(Message::Status(format!(
                                        "Removed {title}, freeing {:.1} GB.",
                                        freed as f64 / 1e9
                                    )));
                                }
                                Err(err) => {
                                    let _ = proxy.send_event(Message::Status(format!(
                                        "Could not remove {title}: {err}"
                                    )));
                                }
                            }
                        }
                    }
                    Request::Report { id } => {
                        let known = failures.lock().unwrap().get(&id).cloned();
                        match known {
                            None => {
                                let _ = proxy.send_event(Message::Status(
                                    "There is nothing to report for that game.".to_string(),
                                ));
                            }
                            Some((reason, log_path)) => {
                                let title = games
                                    .iter()
                                    .find(|game| game.id == id)
                                    .map(|game| game.title.clone())
                                    .unwrap_or_else(|| id.clone());
                                let report = crate::crashreport::build(
                                    &title, &id, &reason, &log_path, &wine, &private,
                                );
                                // The whole thing goes next to the log, so
                                // anything the URL cannot carry is still to
                                // hand.
                                let full = log_path.with_extension("report.md");
                                let _ = std::fs::write(
                                    &full,
                                    format!("# {}\n\n{}", report.title, report.body),
                                );
                                if !crate::crashreport::valid_repo(&report_repo) {
                                    let _ = proxy.send_event(Message::Status(format!(
                                        "\"{report_repo}\" is not an owner/name repository.                                          Report saved to {}",
                                        full.display()
                                    )));
                                    continue;
                                }
                                let url = crate::crashreport::issue_url(&report_repo, &report);
                                match std::process::Command::new("xdg-open").arg(&url).spawn() {
                                    Ok(_) => {
                                        let _ = proxy.send_event(Message::Status(format!(
                                            "Opened a draft issue. Full report: {}",
                                            full.display()
                                        )));
                                    }
                                    Err(err) => {
                                        let _ = proxy.send_event(Message::Status(format!(
                                            "No browser to open ({err}). Report saved to {}",
                                            full.display()
                                        )));
                                    }
                                }
                            }
                        }
                    }
                    Request::CheckUpdate => {
                        let _ = proxy.send_event(Message::Status("Checking…".to_string()));
                        let message = match runtime
                            .block_on(crate::update::check(&client, &report_repo))
                        {
                            Err(err) => format!("Could not check: {err}"),
                            Ok(None) => {
                                format!("{} is the newest build.", env!("CARGO_PKG_VERSION"))
                            }
                            Ok(Some(available)) => {
                                let version = available.version.clone();
                                match runtime.block_on(crate::update::install(&client, &available))
                                {
                                    Ok(_) => format!("Updated to {version}. Restart to use it."),
                                    Err(err) => format!("Could not install {version}: {err}"),
                                }
                            }
                        };
                        let _ = proxy.send_event(Message::Status(message));
                    }
                    Request::OpenLog { id } => {
                        let known = failures.lock().unwrap().get(&id).cloned();
                        match known {
                            Some((_, log_path)) => {
                                let _ = std::process::Command::new("xdg-open")
                                    .arg(&log_path)
                                    .spawn();
                            }
                            None => {
                                let _ = proxy.send_event(Message::Status(
                                    "There is no log for that game yet.".to_string(),
                                ));
                            }
                        }
                    }
                    Request::Install { id } => {
                        let expected = games
                            .iter()
                            .find(|game| game.id == id)
                            .map(|game| game.size)
                            .unwrap_or(0);
                        let dir = PathBuf::from(format!("{games_dir}/{id}"));
                        let title = games
                            .iter()
                            .find(|game| game.id == id)
                            .map(|game| game.title.clone())
                            .unwrap_or_else(|| id.clone());
                        // Downloads fail often; keep the reason.
                        let (log_path, out, err) = open_log(&id, "install");
                        let mut child = match std::process::Command::new(&cli)
                            .args(["streaming", &id, &dir.display().to_string()])
                            .stdout(out)
                            .stderr(err)
                            .spawn()
                        {
                            Ok(child) => child,
                            Err(err) => {
                                let _ = proxy.send_event(Message::Progress {
                                    id: id.clone(),
                                    percent: 0,
                                    have: 0,
                                    total: expected,
                                    rate: 0,
                                    note: format!("could not start: {err}"),
                                });
                                continue;
                            }
                        };

                        // The downloader draws its own progress bars for a
                        // terminal, which there is not one of here, so watch
                        // the directory instead.
                        let proxy = proxy.clone();
                        let watched = dir.clone();
                        let id = id.clone();
                        let failures = failures.clone();
                        std::thread::spawn(move || {
                            let mut previous: Option<(std::time::Instant, u64)> = None;
                            loop {
                                match child.try_wait() {
                                    Ok(Some(_)) | Err(_) => break,
                                    Ok(None) => {}
                                }
                                let have = directory_size(&watched);
                                let percent = (have.min(expected) * 100)
                                    .checked_div(expected)
                                    .unwrap_or(0) as u8;
                                let rate = previous
                                    .and_then(|(when, before)| {
                                        let seconds = when.elapsed().as_secs_f64();
                                        (seconds > 0.0 && have > before)
                                            .then(|| ((have - before) as f64 / seconds) as u64)
                                    })
                                    .unwrap_or(0);
                                previous = Some((std::time::Instant::now(), have));
                                let _ = proxy.send_event(Message::Progress {
                                    id: id.clone(),
                                    percent,
                                    have,
                                    total: expected,
                                    rate,
                                    note: String::new(),
                                });
                                std::thread::sleep(std::time::Duration::from_secs(2));
                            }
                            if watched.join(".xodus-streaming.msixvc").exists() {
                                let _ = proxy.send_event(Message::Installed { id });
                            } else {
                                // The downloader stopped without leaving its
                                // marker behind, so say so instead of sitting
                                // on a progress bar that will never move -
                                // and keep the log, which will say why.
                                let _ = proxy.send_event(Message::Progress {
                                    id: id.clone(),
                                    percent: 0,
                                    have: directory_size(&watched),
                                    total: expected,
                                    rate: 0,
                                    note: "the download stopped before it finished".to_string(),
                                });
                                let log = std::fs::read_to_string(&log_path).unwrap_or_default();
                                let reason = crate::crashreport::diagnose(Some(1), &log, 0)
                                    .unwrap_or_else(|| "the download did not finish".to_string());
                                failures
                                    .lock()
                                    .unwrap()
                                    .insert(id.clone(), (reason.clone(), log_path.clone()));
                                let _ = proxy.send_event(Message::Crashed {
                                    id,
                                    title,
                                    reason,
                                });
                            }
                        });
                    }
                }
            }
        });
    }

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(Message::Account {
                gamertag,
                signed_in,
            }) => {
                let _ = webview.evaluate_script(&format!(
                    "xodusAccount({}, {signed_in})",
                    serde_json::to_string(&gamertag).unwrap_or_default()
                ));
            }
            Event::UserEvent(Message::Settings {
                games_dir,
                wine,
                prefix,
                market,
                report_repo,
                catalogue,
                refreshed,
            }) => {
                let payload = serde_json::json!({
                    "games_dir": games_dir,
                    "wine": wine,
                    "prefix": prefix,
                    "market": market,
                    "report_repo": report_repo,
                    "catalogue": catalogue,
                    "refreshed": refreshed,
                    "version": env!("CARGO_PKG_VERSION"),
                });
                let _ = webview.evaluate_script(&format!("xodusSettings({payload})"));
            }
            Event::UserEvent(Message::Crashed { id, title, reason }) => {
                let _ = webview.evaluate_script(&format!(
                    "xodusCrashed({}, {}, {})",
                    serde_json::to_string(&id).unwrap_or_default(),
                    serde_json::to_string(&title).unwrap_or_default(),
                    serde_json::to_string(&reason).unwrap_or_default()
                ));
            }
            Event::UserEvent(Message::Running { id, running }) => {
                let _ = webview.evaluate_script(&format!(
                    "xodusRunning({}, {running})",
                    serde_json::to_string(&id).unwrap_or_default()
                ));
            }
            Event::UserEvent(Message::Ready) => {
                let _ = webview.evaluate_script("xodusLoaded()");
            }
            Event::UserEvent(Message::Status(text)) => {
                let _ = webview.evaluate_script(&format!(
                    "xodusStatus({})",
                    serde_json::to_string(&text).unwrap_or_default()
                ));
            }
            Event::UserEvent(Message::Catalogue(games)) => {
                let json = serde_json::to_string(&games).unwrap_or_else(|_| "[]".into());
                let _ = webview.evaluate_script(&format!("xodusCatalogue({json})"));
            }
            Event::UserEvent(Message::Progress {
                id,
                percent,
                have,
                total,
                rate,
                note,
            }) => {
                let _ = webview.evaluate_script(&format!(
                    "xodusProgress({}, {percent}, {have}, {total}, {rate}, {})",
                    serde_json::to_string(&id).unwrap_or_default(),
                    serde_json::to_string(&note).unwrap_or_default()
                ));
            }
            Event::UserEvent(Message::Installed { id }) => {
                let _ = webview.evaluate_script(&format!(
                    "xodusInstalled({})",
                    serde_json::to_string(&id).unwrap_or_default()
                ));
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{runs_from, safe_id};

    /// Both spellings of the same executable, taken from real command lines.
    #[test]
    fn a_game_is_recognised_however_wine_spells_its_path() {
        let dir = "/home/player/Games/xpedited/9NGLST31DG26";
        assert!(runs_from("/home/player/Games/xpedited/9NGLST31DG26/DiggingGame.exe", dir));
        assert!(runs_from(
            "Z:\\home\\player\\Games\\xpedited\\9NGLST31DG26\\DiggingGame\\Binaries\\WinGDK\\DiggingGame-WinGDK-Shipping.exe",
            dir
        ));
        assert!(runs_from(
            "C:\\home\\player\\Games\\xpedited\\9NGLST31DG26\\Game.exe",
            dir
        ));
    }

    #[test]
    fn other_processes_are_left_alone() {
        let dir = "/home/player/Games/xpedited/9NGLST31DG26";
        // The launcher: it mentions the folder, but is not inside it.
        assert!(!runs_from("/home/player/.local/bin/xpedited", dir));
        // A different game whose id merely starts the same way.
        assert!(!runs_from("/home/player/Games/xpedited/9NGLST31DG26X/Game.exe", dir));
        // The folder itself is not a program in it.
        assert!(!runs_from("/home/player/Games/xpedited/9NGLST31DG26", dir));
        assert!(!runs_from("", dir));
    }

    #[test]
    fn store_ids_are_accepted() {
        assert!(safe_id("9MTVJ3HHTQGS"));
        assert!(safe_id("9NBLGGH4R0GC"));
    }

    /// Uninstalling deletes a directory recursively, so the id it is handed
    /// has to be one the catalogue could have produced and nothing else.
    #[test]
    fn anything_that_could_escape_the_games_folder_is_refused() {
        for bad in [
            "",
            "..",
            "../..",
            "9MTV/../..",
            "9MTV/sub",
            "/etc",
            "a b",
            ".",
            "with-dash",
            "with_underscore",
            "with.dot",
        ] {
            assert!(!safe_id(bad), "{bad:?} should have been refused");
        }
        assert!(!safe_id(&"9".repeat(64)));
    }
}
