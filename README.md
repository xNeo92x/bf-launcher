# BF Launcher

**English** · [Deutsch](README.de.md)

A Linux launcher written in Rust for **Battlefield 3** and **Battlefield 4**, built for CachyOS and Arch.

- **Setup and sign-in:** The launcher installs the official EA app in its own Proton prefix (via `umu-launcher`) and starts it. You sign in inside the EA app window. The launcher never sees or stores your credentials.
- **Game selection:** For BF3 and BF4 the launcher shows the status from your EA library. **Download & install** starts the download in the EA app, and the progress shows up in the launcher.
- **Server browser:** Once a game is installed, **Server browser** opens the game's real Battlelog (`/bf3/<lang>/servers/` or `/bf4/<lang>/servers/`) inside the launcher window. You join servers as usual, and the game starts through the EA app.
- **Real player counts:** In the server list, the real number of connected players appears after the count Battlelog reports, for example `38 [6] / 48`. The colour shows how much the number is inflated. The bar in the bottom-left corner has the options **real count only** and **hide empty**. The idea comes from the BBLog plugin [bblog-true-players-bf3](https://github.com/Dendari92/bblog-true-players-bf3). The launcher uses its own implementation, which also supports BF4.
- **Language:** The launcher picks German if your system language is German, and English otherwise. You can override this in the settings.
- **Automatic updates:** Every change pushed to this repository is built automatically. The launcher notices the new build and can update itself with one click.

## Installation

### Prebuilt binary

```bash
sudo pacman -S --needed webkit2gtk-4.1 umu-launcher
mkdir -p ~/.local/bin
curl -L -o ~/.local/bin/bf-launcher https://github.com/xNeo92x/bf-launcher/releases/download/latest/bf-launcher-x86_64-linux
chmod +x ~/.local/bin/bf-launcher
```

### Build from source

```bash
sudo pacman -S --needed rust webkit2gtk-4.1 umu-launcher
git clone https://github.com/xNeo92x/bf-launcher.git
cd bf-launcher
./install.sh
```

`install.sh` also adds **BF Launcher** to the application menu. You can start it from there or by running `bf-launcher`.

## First start

1. **Install EA app:** The launcher downloads the official installer from EA and runs it in the prefix. Follow the installer window.
2. **Start EA app** and sign in there. Once the sign-in is detected, the game selection appears.
3. Click **Download & install** for BF3 or BF4. Depending on the EA app version, you may have to confirm the install folder in the EA app.
4. After the installation, click **Server browser**. The first time, you sign in to Battlelog on EA's sign-in page. The session is saved afterwards.

## Updates

On every push to `main`, GitHub Actions builds the launcher. It then publishes the binary, a SHA-256 checksum and `version.json` (the commit) in the [`latest`](https://github.com/xNeo92x/bf-launcher/releases/tag/latest) release.

The launcher compares the commit it was built from with that release. It checks at startup and then every 30 minutes, or right away via **Settings → Check for updates now**. If the commits differ, a banner offers **Update now**. The launcher downloads the new build, verifies the checksum and the ELF header, and replaces its own binary atomically. After **Restart**, the new version runs.

Automatic checking can be turned off in the settings. The launcher's binary must be writable by your user, for example in `~/.local/bin`.

## How it works

```
┌──────────── BF Launcher (Rust: tao + wry/WebKitGTK) ─────────────┐
│ bfl://  launcher UI (embedded, strict CSP, no network)           │
│ https://battlelog.battlefield.com  + injected script:            │
│     • real player counts (getPlayersOnServer)                    │
│     • bar "← Launcher / BF3 / BF4"                               │
│     • bridge: jQuery requests to 127.0.0.1:3215/4219 ──IPC──┐    │
│ Rust backend ◄──────────────────────────────────────────────┘    │
│     • checks target and path (allowlist) and forwards            │
│     • queries status and downloads from the EA app               │
│     • update check against GitHub releases                       │
└───────────────┬──────────────────────────────────────────────────┘
                │ http://127.0.0.1:3215 (local EA app API)
        EA app in Proton (umu-run, own prefix) ──► bf3.exe / bf4.exe
```

Battlelog starts games through a local HTTP API of the EA app (`/game/launch`, `/game/status` …). Under Proton this API is also reachable from the Linux host. WebKitGTK, however, blocks requests from an HTTPS page to `http://127.0.0.1` as mixed content. That is why the launcher hooks a jQuery transport into Battlelog. The transport passes exactly these requests to the Rust backend, and the backend carries them out.

## Security

- **No credentials in the launcher:** Sign-in happens only in the EA app and on EA's own sign-in page.
- **Separate IPC permissions:** Commands are separated by their origin.
  - The launcher UI (`bfl://localhost`) may trigger launcher actions.
  - Battlelog may only do three things: go back to the launcher, switch game, and make requests to the EA app.
  - EA app requests from Battlelog go through a strict allowlist: only `127.0.0.1:3215` with the paths `/ping`, `/game`, `/library`, `/store` and `/version`, plus `127.0.0.1:4219/`. At most 24 requests run at once, and responses are limited to 2 MB.
  - All messages are parsed with strict `serde` types (`deny_unknown_fields`).
- **Strict CSP for the launcher UI:** no external content and no inline code.
- **Navigation:** `file:` and other foreign schemes are blocked. Popups open in the system browser, but only `http(s)`.
- **Processes:** Programs are started without a shell, with fixed argument lists. Settings are validated before they are saved.
- **Downloads:** The EA app installer comes only over HTTPS from the official EA CDN. Updates come only over HTTPS from GitHub releases and are checked against their SHA-256 checksum.

## Files

| Path | Contents |
|---|---|
| `~/.config/bf-launcher/config.toml` | Settings (prefix, Proton, languages, updates) |
| `~/.local/share/bf-launcher/prefix` | Proton prefix with the EA app and games |
| `~/.local/share/bf-launcher/webview` | Web profile (Battlelog sign-in) |
| `~/.local/share/bf-launcher/logs` | Logs of the EA app and the installer |

By default the launcher uses the newest installed **GE-Proton**. Alternatively, enter `GE-Proton` and umu downloads it itself.

## Diagnostics

```bash
RUST_LOG=bf_launcher=debug bf-launcher          # verbose logs
BFL_DEBUG=1 bf-launcher                         # WebKit developer tools (right click → Inspect)
BFL_SELFTEST=bf4 bf-launcher                    # checks player counts and the EA app bridge on Battlelog
bf-launcher --version | --check-update | --update   # version / update from the command line
```

## Notes and limits

- **Playability under Proton:** Whether BF3/BF4 run depends on Proton and the anti-cheat (PunkBuster/FairFight or EA Javelin). EA can change this at any time, and the launcher cannot work around it.
- **Browser identification:** Battlelog only allows joining a game from Windows browsers (server-side check, dialog "Unable to launch the game from this device"). The game runs as a Windows program under Proton anyway, so the embedded browser identifies itself as Firefox on Windows (32-bit).
- **Battlelog rate limit:** Battlelog blocks an IP for a while (HTTP 429) if it sends too many requests. While blocked, Battlelog itself fails as well, for example the region filter. The launcher therefore fetches real player counts only for visible servers, at most about 15 per minute, caches them for 2 minutes and pauses automatically on HTTP 429.
- **Unofficial endpoints:** Battlelog endpoints such as `getPlayersOnServer` and the local EA app API are not officially documented and may change.
- **Licence of the original plugin:** The BBLog plugin by Dendari92 has no licence. The launcher therefore contains no code from it, only its own implementation of the same idea.

## License

[MIT](LICENSE)
