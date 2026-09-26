<p align="center"><img width="128" src="assets/xpedited.png" /></p>
<h1 align="center">Xpedited</h1>
<p align="center">A fork of <a href="https://github.com/xodus-gaming/xodus">Xodus</a> - the great gaming migration to Linux</p>
<p align="center">
    <a href="https://discord.gg/ZG774FK4tq">
        <img src="https://img.shields.io/discord/1123890623586504714?logo=discord&style=for-the-badge&color=red&label=Upstream+Discord" alt="Upstream Discord" />
    </a>
</p>

> [!IMPORTANT]
> **Xpedited is a fork. The original project is [Xodus](https://github.com/xodus-gaming/xodus) and all credit belongs there.**
> The changes in this fork are LLM-assisted, which upstream does not accept, so it stays downstream and
> nothing here should be sent to them. See [FORK.md](FORK.md) for why, and please go and support the
> original project rather than this copy of it.

> [!CAUTION]
> This is an unofficial project - use at your own risk. It is not affiliated with, endorsed by, or sponsored by Microsoft or XBOX; all trademarks, product names, and company names or logos mentioned herein are the property of their respective owners.

## What this fork adds

Upstream Xodus is the engine: it signs in, downloads MSIXVC packages, handles
licences and decrypts executables. Xpedited keeps all of that and adds the
parts you touch.

- **A desktop app.** One window to browse the PC Game Pass catalogue, see what
  you own, install a game and play it. Sign in, settings, install progress,
  stop a running game, uninstall.
- **Crash reports.** When a game fails, the app keeps the log, works out why in
  one line, and can open a pre-filled GitHub issue. Paths, gamertag, XUID,
  tokens and email addresses are redacted first, and you see the report before
  it goes anywhere.
- **Xbox Game Services in Wine.** `XGameSave`, `XAsync`/`XTaskQueue`, `XUser`,
  `XPackage`, `XStore`, `XSystem`, `XNetworking`.
- **Exports to other launchers.** `xpedited heroic`, `heroic-catalog` and
  `steam` add the games with their store art; `steam --remove` takes them out.
- **Self-update.** `xpedited update` checks GitHub releases and replaces the
  binary in place.

### Not working yet

- **Multiplayer, achievements and presence.** `XUserGetTokenAndSignature` now
  mints a token for the service being called, but requests are unsigned.
- **A black-screen group.** ~19 titles run and draw nothing. Confirmed on a real
  display, not a test artifact, and inconsistent run to run.
- **Direct3D 12 / shader model 6.** vkd3d does not expose SM6, so some titles
  refuse to start and Godot 4's D3D12 shaders crash its DXIL parser.
- **MSIXVC2 packages**, same as upstream.

## Installing

You need a patched Wine build with the `xgameruntime` work in it, and Rust.

```sh
git clone <this repo> xpedited && cd xpedited
./install.sh
```

That builds the binary, puts it in `~/.local/bin`, and adds a menu entry and
icon. Start it once with the path to your patched Wine, which is remembered in
`~/.config/xpedited/settings.json`:

```sh
xpedited app /path/to/patched/wine
```

After that `xpedited app` on its own is enough. `./uninstall.sh` reverses it.

## Using the app

Sign in from the account button; the Microsoft login opens in its own window.
Click a game to install it, and the same button plays it. Behind the account
button are settings for the games folder, Wine binary, prefix, region and the
repository crash reports are sent to.

## Privacy

- Your account lives in the system keyring, never in a file in this repo.
- The service socket is owner-only.
- Crash reports redact your home directory, username, gamertag, XUID, anything
  that looks like a token, and email addresses — and are shown to you as a
  draft GitHub issue before anything is sent.

## Upstream's state of the project

The section below is from Xodus and describes the engine this fork is built on.

The project can now login, download packages and obtain licenses for games.

These parts are still quite scattered arround.

- [x] Device login
- [x] User login
- [x] XBOX authorization
- [x] MSIXVC download
- [x] On-demand .exe decryption [#50](https://github.com/xodus-gaming/xodus/issues/50)
- [ ] MSIXVC2 support [#53](https://github.com/xodus-gaming/xodus/issues/53)

## FAQ

**Q: What is Xodus**  
Xodus aims to bring XBOX PC games to Linux and possibly Mac devices.

**Q: When can I play my Minecraft Bedrock?**  
While Xodus is quickly maturing, there is still a lot of work to support it from Wine standpoint to provide necessary XBOX Services to games.  
_TL;DR_ soon<sup>tm</sup>

**Q: How to get involved?**  
Start by joining our Discord or review any open GitHub issues .

**Q: What games will be supported?**  
We hope to manage to support most of the catalog, the limitation is the game has to be GDK and in MSIXVC format.  
So far `Gears of War 4` is a prominent unsupported title for the time being.

**Q: Will XBOX Backward Compatibility on PC work?**  
While Xodus is capable of downloading and running those titles. It's possible these games will work only after additional patches to wine, dxvk or vkd3d-proton.

## Building

The project structure is as follows.

```
.
├── msixvc - [rlib] common rlib crate for utilities for parsing MSIXVC and XSP files
├── xodus - [rlib] common rlib crate that contains core xodus functionality, API calls abstractions and utilities
├── xodus-cli - [bin] the CLI and the app; builds the `xpedited` binary
└── xodus-service - [bin] service process exposing a xodus.sock for IPC communication, it takes care of xgameruntime.dll integration.
```

> [!NOTE]
> xodus-service aims to become a main point of integration. All xodus clients will connect to it to interact with games and XBOX services.

### Prerequisites

- Rust version 1.98 or later
- Right now CLI relies on wry and tao to show a login page. Consult https://docs.rs/wry/latest/wry/#platform-considerations
- xodus-service relies on `protoc` to compile `proto/` definitions make sure to install it for your platform

### Running

Building all crates in release mode

```bash
cargo build --release --workspace
```

Running cli in debug

```
cargo run -- --help
```

Running xodus-service in debug

```
cargo run --bin xodus-service
```

Debug and profile `xpedited` or `xodus-service` with [tokio-console]([tokio-console](https://github.com/tokio-rs/console))

```
RUSTFLAGS="--cfg tokio_unstable" cargo run --features tokio_console 
```

> [!WARNING]
> For better performance when decrypting MSIXVC files, the `aes` and `ssse3` features are enabled on `x86_64`,
> and the `aes` feature is enabled on `aarch64`. This means that the program will crash with an illegal instruction
> error when running on a CPU which doesn't support those instructions.
>
> See https://en.wikipedia.org/wiki/AES_instruction_set for a list of compatible CPUs (every processor from
> 2011 onwards should be supported).

### CLI Usage

```
Xbox Store and Game Pass games on Linux. A fork of Xodus.

Usage: xpedited <COMMAND>

Commands:
  download        Download msixvc or xsp files fo given game
  license         Dump CIKs for use with XvdTool
  extract         Extract locally stored msixvc file
  extract-eappx   Extract a locally stored EAppx/EMSIX package (research task, see issue #91)
  library         List games on your account
  app             Open the Xpedited window: browse, install and play
  steam           Add the games you have downloaded to Steam, with their store art
  heroic-sync     Re-sort Heroic's Xbox categories by what is actually downloaded
  heroic-catalog  Put the whole PC Game Pass catalogue in the Heroic library
  heroic          Add an extracted game to the Heroic Games Launcher library
  metadata        Show a product's title, description and store art
  update          Check for a newer version and install it
  login           Sign in to your Microsoft account
  logout          Forget the signed in account
  streaming       Download and extract the game through streaming algorithm
  run             Run a Game with xodus wine
  clep            Generate or decrypt base64-encoded CLEP challenge data
  sp-license      Decode SPLicenseBlock
  help            Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

## Special Thanks

- [XvdTool.Streaming](https://github.com/LukeFZ/XvdTool.Streaming) and [CikExtractor](https://github.com/LukeFZ/CikExtractor) by LukeFZ
