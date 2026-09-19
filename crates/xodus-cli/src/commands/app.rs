//! A window over everything the other commands do: browse the catalogue,
//! install, play. The interface is a webview because tao and wry are already
//! here for the sign in flow, and because a grid of box art is a thing HTML
//! is good at.

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
}

/// What the window is told, from whichever thread has something to say.
enum Message {
    Catalogue(Vec<Game>),
    Progress {
        id: String,
        percent: u8,
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

fn cache_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(format!("{home}/.cache/xodus/catalogue.json"))
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
    games.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));

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

pub async fn run(
    client: &reqwest::Client,
    tokens: &xodus::tokens::TokenManager,
    wine: String,
    games_dir: Option<String>,
    market: Option<String>,
    prefix: Option<String>,
    refresh: bool,
) -> ExitCode {
    let home = std::env::var("HOME").unwrap_or_default();
    let games_dir = games_dir.unwrap_or_else(|| format!("{home}/Games/xodus"));
    let market = market.unwrap_or("US".to_string());
    let prefix = prefix.unwrap_or_else(|| format!("{home}/.local/share/xodus/prefix"));
    let cli = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "xodus-cli".to_string());

    // The account's own titles need the caller's credentials, so they are
    // gathered here rather than on the worker thread.
    let owned = load_owned(client, tokens, &market, &games_dir).await;
    if !owned.is_empty() {
        println!("{} titles on this account", owned.len());
    }

    let event_loop = EventLoopBuilder::<Message>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = match WindowBuilder::new()
        .with_title("Xodus")
        .with_inner_size(LogicalSize::new(1180.0, 760.0))
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
        let games_dir = games_dir.clone();
        let market = market.clone();
        let wine = wine.clone();
        let prefix = prefix.clone();
        let cli = cli.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().expect("a runtime");
            let client = reqwest::Client::builder()
                .user_agent(concat!("xodus-cli/", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("a client");

            let mut games = runtime.block_on(load_catalogue(&client, &market, &games_dir, refresh));

            // Owned titles take precedence: a game can be both, and owning
            // it is the more useful thing to be told.
            for game in owned {
                match games.iter_mut().find(|other| other.id == game.id) {
                    Some(existing) => existing.source = "owned".to_string(),
                    None => games.push(game),
                }
            }
            games.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
            let _ = proxy.send_event(Message::Catalogue(games.clone()));

            while let Ok(request) = incoming.recv() {
                match request {
                    Request::Ready => {
                        for game in &mut games {
                            game.installed = installed_at(&games_dir, &game.id);
                        }
                        let _ = proxy.send_event(Message::Catalogue(games.clone()));
                    }
                    Request::Play { id } => {
                        let dir = format!("{games_dir}/{id}");
                        let _ = std::process::Command::new(&cli)
                            .args(["run", &dir, &wine])
                            .env("WINEPREFIX", &prefix)
                            .spawn();
                    }
                    Request::Install { id } => {
                        let expected = games
                            .iter()
                            .find(|game| game.id == id)
                            .map(|game| game.size)
                            .unwrap_or(0);
                        let dir = PathBuf::from(format!("{games_dir}/{id}"));
                        let mut child = match std::process::Command::new(&cli)
                            .args(["streaming", &id, &dir.display().to_string()])
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .spawn()
                        {
                            Ok(child) => child,
                            Err(err) => {
                                let _ = proxy.send_event(Message::Progress {
                                    id: id.clone(),
                                    percent: 0,
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
                        std::thread::spawn(move || {
                            loop {
                                match child.try_wait() {
                                    Ok(Some(_)) | Err(_) => break,
                                    Ok(None) => {}
                                }
                                let have = directory_size(&watched);
                                let percent = if expected > 0 {
                                    ((have.min(expected) * 100) / expected) as u8
                                } else {
                                    0
                                };
                                let _ = proxy.send_event(Message::Progress {
                                    id: id.clone(),
                                    percent,
                                    note: format!("{:.1} GB", have as f64 / 1e9),
                                });
                                std::thread::sleep(std::time::Duration::from_secs(2));
                            }
                            if watched.join(".xodus-streaming.msixvc").exists() {
                                let _ = proxy.send_event(Message::Installed { id });
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
            Event::UserEvent(Message::Catalogue(games)) => {
                let json = serde_json::to_string(&games).unwrap_or_else(|_| "[]".into());
                let _ = webview.evaluate_script(&format!("xodusCatalogue({json})"));
            }
            Event::UserEvent(Message::Progress { id, percent, note }) => {
                let _ = webview.evaluate_script(&format!(
                    "xodusProgress({}, {percent}, {})",
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
