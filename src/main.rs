//! BF Launcher – Battlefield 3 & 4 unter CachyOS/Linux.
//!
//! Aufbau:
//! * Ein natives Fenster (tao) mit einer WebKitGTK-Ansicht (wry).
//! * Die Launcher-Oberfläche wird über das eigene Schema `bfl://` aus dem
//!   Programm selbst ausgeliefert (keine Netzwerkzugriffe, strikte CSP).
//! * Der Serverbrowser ist das echte Battlelog; ein eingebettetes Skript
//!   ergänzt die echten Spielerzahlen und eine Leiste "Zurück zum Launcher".
//! * Spielstart und Installation laufen über die EA App, die per umu-launcher
//!   und Proton in einem eigenen Präfix läuft.

mod config;
mod ea_api;
mod games;
mod i18n;
mod runner;
mod updater;

use anyhow::Result;
use config::Config;
use ea_api::{EaApi, EaState};
use games::Game;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tao::dpi::LogicalSize;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::WindowBuilder;
use wry::http::{Request, Response, header};
use wry::{NewWindowResponse, WebContext, WebViewBuilder};

const LAUNCHER_URL: &str = "bfl://localhost/";
/// Battlelog erlaubt den Spielstart serverseitig nur für Windows-Browser (32-Bit-Kennung,
/// siehe Dialog "Unable to launch the game from this device"). Das Spiel läuft unter Proton
/// ohnehin als Windows-Programm, daher meldet sich die WebView als Firefox unter Windows.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; rv:140.0) Gecko/20100101 Firefox/140.0";
const BATTLELOG_HOST: &str = "battlelog.battlefield.com";

const INDEX_HTML: &str = include_str!("../ui/index.html");
const APP_JS: &str = include_str!("../ui/app.js");
const STYLE_CSS: &str = include_str!("../ui/style.css");
const BATTLELOG_JS: &str = include_str!("../inject/battlelog.js");

// ---------------------------------------------------------------------------
// Zustand, der an die Oberfläche gesendet wird
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Default)]
struct GameInfo {
    /// unknown | not_owned | not_installed | downloading | installed
    status: String,
    state: String,
    progress: f64,
    offer_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct Task {
    label: String,
    /// 0..1, oder None für unbestimmt
    progress: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct UiState {
    umu: bool,
    ea_installed: bool,
    ea: EaState,
    games: BTreeMap<Game, GameInfo>,
    task: Option<Task>,
    error: Option<String>,
    notice: Option<String>,
    settings: Settings,
    lang: &'static str,
    version: String,
    update: Option<updater::UpdateInfo>,
    update_ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Settings {
    prefix: String,
    proton: String,
    locale: String,
    true_players: bool,
    language: String,
    check_updates: bool,
}

impl From<&Config> for Settings {
    fn from(c: &Config) -> Self {
        Self {
            prefix: c.prefix.to_string_lossy().into_owned(),
            proton: c.proton.clone(),
            locale: c.locale.clone(),
            true_players: c.true_players,
            language: c.language.clone(),
            check_updates: c.check_updates,
        }
    }
}

struct Shared {
    cfg: Config,
    task: Option<Task>,
    error: Option<String>,
    notice: Option<String>,
    ea: EaState,
    games: BTreeMap<Game, GameInfo>,
    update: Option<updater::UpdateInfo>,
    update_ready: bool,
}

type SharedRef = Arc<Mutex<Shared>>;

fn snapshot(s: &Shared) -> UiState {
    UiState {
        umu: runner::umu_available(),
        ea_installed: runner::ea_app_installed(&s.cfg),
        ea: s.ea,
        games: s.games.clone(),
        task: s.task.clone(),
        error: s.error.clone(),
        notice: s.notice.clone(),
        settings: Settings::from(&s.cfg),
        lang: i18n::lang().code(),
        version: format!("{} ({})", updater::VERSION, updater::short(updater::BUILD_COMMIT)),
        update: s.update.clone(),
        update_ready: s.update_ready,
    }
}

// ---------------------------------------------------------------------------
// IPC
// ---------------------------------------------------------------------------

/// Befehle der Launcher-Oberfläche (nur von `bfl://` akzeptiert).
#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case", deny_unknown_fields)]
enum UiCommand {
    Ready,
    InstallEaApp,
    StartEaApp,
    StopEaApp,
    OpenEaLibrary,
    InstallGame { game: Game },
    OpenServerBrowser { game: Game },
    OpenInFirefox { game: Game },
    SaveSettings { settings: Settings },
    OpenLogs,
    DismissMessage,
    CheckUpdates,
    ApplyUpdate,
    RestartLauncher,
}

/// Befehle, die die Battlelog-Seite senden darf (bewusst minimal).
#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case", deny_unknown_fields)]
enum BattlelogCommand {
    BackToLauncher,
    SwitchGame { game: Game },
    OpenInFirefox { game: Game },
    /// Weiterleitung einer Anfrage an die EA App (siehe ea_api::bridge_target).
    EaRequest {
        id: u64,
        method: String,
        url: String,
        body: Option<String>,
        timeout: Option<u64>,
    },
}

enum UserEvent {
    Push,
    Ui(UiCommand),
    Battlelog(BattlelogCommand),
    External(String),
    PageLoaded(String),
    SelftestCollect,
    EaReply { id: u64, resp: ea_api::BridgeResponse },
}

/// Begrenzung gleichzeitiger Brücken-Anfragen.
static BRIDGE_INFLIGHT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
const BRIDGE_MAX: usize = 24;

/// Diagnose (BFL_SELFTEST=1): prüft auf Battlelog, ob die EA-App-API aus der
/// Seite erreichbar ist und ob das Spielerzahl-Skript arbeitet.
const SELFTEST_JS: &str = r#"(async () => {
  const out = { url: location.href };
  out.ping = await new Promise(res => jQuery.ajax("http://127.0.0.1:3215/ping", { dataType: "json", timeout: 3000 })
      .done(d => res("OK " + JSON.stringify(d))).fail((x, s) => res("FEHLER " + x.status + " " + s)));
  out.loggedIn = await new Promise(res => jQuery.ajax("http://127.0.0.1:3215/game/status?masterTitleId=000000", { dataType: "json" })
      .done(() => res("done?")).fail(x => res("fail " + x.status + " " + x.responseText)));
  out.blocked = await new Promise(res => jQuery.ajax("http://127.0.0.1:3215/admin").done(() => res("DURCHGELASSEN")).fail(x => res("blockiert " + x.status)));
  await new Promise(r => setTimeout(r, 8000));
  out.bar = !!document.getElementById("bfl-bar");
  out.real = [...document.querySelectorAll(".bfl-real")].slice(0, 8).map(e => e.closest("td,div").innerText.replace(/\s+/g, " ").trim());
  out.realCount = document.querySelectorAll(".bfl-real").length;
  out.realDone = document.querySelectorAll(".bfl-real:not(.bfl-wait)").length;
  out.playerRequests = performance.getEntriesByType("resource").filter(e => e.name.includes("getPlayersOnServer")).length;
  try {
    const g = location.pathname.split("/")[1], lang = location.pathname.split("/")[2];
    const r = await fetch(`/${g}/${lang}/servers/getServerRegions/pc/`, { headers: { "X-Requested-With": "XMLHttpRequest" } });
    out.regions = r.status;
    const ctx = window.Surface && Surface.globalContext;
    out.loggedInBattlelog = !!(ctx && ctx.session && ctx.session.isLoggedIn);
    out.isWindows = !!(ctx && ctx.staticContext.detectedBrowser.isWindows);
    out.isWin64 = !!(ctx && ctx.staticContext.detectedBrowser.isWin64);
    if (out.loggedInBattlelog) {
      const pr = await fetch(`/${g}/launcher/playablepersona/`, { headers: { "X-Requested-With": "XMLHttpRequest" } });
      const pj = await pr.json().catch(() => ({}));
      out.allowRunGame = pj && pj.data ? pj.data.allowRunGame : "status " + pr.status;
    }
  } catch (e) { out.regions = "FEHLER " + e; }
  window.__bflSelftest = JSON.stringify(out);
})(); 0"#;

fn origin_of(uri: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = uri.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.split(':').next()?;
    Some((scheme, host))
}

fn is_launcher(uri: &str) -> bool {
    matches!(origin_of(uri), Some(("bfl", "localhost")))
}

fn is_battlelog(uri: &str) -> bool {
    matches!(origin_of(uri), Some(("https", BATTLELOG_HOST)))
}

// ---------------------------------------------------------------------------
// Eigenes Schema bfl:// – liefert die eingebettete Oberfläche aus
// ---------------------------------------------------------------------------

fn serve(req: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let (body, mime): (Cow<'static, str>, &str) = match req.uri().path() {
        "/" | "/index.html" => (
            Cow::Owned(INDEX_HTML.replacen("<html lang=\"de\">", &format!("<html lang=\"{}\">", i18n::lang().code()), 1)),
            "text/html; charset=utf-8",
        ),
        "/app.js" => (Cow::Borrowed(APP_JS), "text/javascript; charset=utf-8"),
        "/style.css" => (Cow::Borrowed(STYLE_CSS), "text/css; charset=utf-8"),
        _ => {
            return Response::builder()
                .status(404)
                .body(Cow::Borrowed(&b"not found"[..]))
                .unwrap();
        }
    };
    Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
             font-src 'self'; connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'",
        )
        .header("X-Content-Type-Options", "nosniff")
        .body(match body {
            Cow::Borrowed(b) => Cow::Borrowed(b.as_bytes()),
            Cow::Owned(b) => Cow::Owned(b.into_bytes()),
        })
        .unwrap()
}

// ---------------------------------------------------------------------------
// Hintergrundarbeit
// ---------------------------------------------------------------------------

fn set_task(shared: &SharedRef, proxy: &EventLoopProxy<UserEvent>, task: Option<Task>) {
    shared.lock().unwrap().task = task;
    let _ = proxy.send_event(UserEvent::Push);
}

fn set_result(shared: &SharedRef, proxy: &EventLoopProxy<UserEvent>, res: Result<String>) {
    {
        let mut s = shared.lock().unwrap();
        s.task = None;
        match res {
            Ok(msg) => {
                s.error = None;
                s.notice = (!msg.is_empty()).then_some(msg);
            }
            Err(e) => {
                log::error!("{e:#}");
                s.error = Some(format!("{e:#}"));
            }
        }
    }
    let _ = proxy.send_event(UserEvent::Push);
}

/// Führt eine längere Aufgabe in einem eigenen Thread aus (immer nur eine gleichzeitig).
fn spawn_task(
    shared: &SharedRef,
    proxy: &EventLoopProxy<UserEvent>,
    label: &str,
    f: impl FnOnce(Config, &dyn Fn(Option<f64>)) -> Result<String> + Send + 'static,
) {
    {
        let mut s = shared.lock().unwrap();
        if s.task.is_some() {
            return;
        }
        s.error = None;
        s.notice = None;
    }
    let label = label.to_string();
    set_task(shared, proxy, Some(Task { label: label.clone(), progress: None }));
    let (shared, proxy) = (shared.clone(), proxy.clone());
    std::thread::spawn(move || {
        let cfg = shared.lock().unwrap().cfg.clone();
        let report = |p: Option<f64>| set_task(&shared, &proxy, Some(Task { label: label.clone(), progress: p }));
        let res = f(cfg, &report);
        set_result(&shared, &proxy, res);
    });
}

/// Ordnet den Zustand der EA App einer Anzeige zu. Nur aktive Zustände zählen als Download.
fn game_status(o: &ea_api::Offer, state: &str) -> &'static str {
    if o.is_installed() {
        "installed"
    } else if matches!(state, "NOT_PURCHASED" | "UNRELEASED") {
        "not_owned"
    } else if matches!(
        state,
        "DOWNLOADING"
            | "INSTALLING"
            | "UPDATING"
            | "PREPARING"
            | "PAUSED"
            | "QUEUED"
            | "BUSY"
            | "READY_TO_INSTALL"
            | "FINALIZING_DOWNLOAD"
            | "READY_TO_UNPACK"
            | "UNPACKING"
            | "READY_TO_DECRYPT"
            | "DECRYPTING"
    ) {
        "downloading"
    } else {
        // NOT_INSTALLED, READY_TO_DOWNLOAD, UNKNOWN, …
        "not_installed"
    }
}

fn poll_status(api: &EaApi) -> (EaState, BTreeMap<Game, GameInfo>) {
    let ea = api.state();
    let mut games = BTreeMap::new();
    for g in Game::ALL {
        let info = if ea != EaState::LoggedIn {
            GameInfo { status: "unknown".into(), ..Default::default() }
        } else {
            match api.main_offer(g) {
                Ok(Some(o)) => {
                    let state = o.state();
                    let status = game_status(&o, &state);
                    GameInfo {
                        status: status.into(),
                        state,
                        progress: o.progress.clamp(0.0, 1.0),
                        offer_id: Some(o.offer_id),
                    }
                }
                Ok(None) => GameInfo { status: "not_owned".into(), ..Default::default() },
                Err(e) => {
                    log::debug!("Status {g:?}: {e:#}");
                    GameInfo { status: "unknown".into(), ..Default::default() }
                }
            }
        };
        games.insert(g, info);
    }
    (ea, games)
}

fn start_poller(shared: SharedRef, proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        let api = EaApi::new();
        loop {
            let (ea, games) = poll_status(&api);
            {
                let mut s = shared.lock().unwrap();
                s.ea = ea;
                s.games = games;
            }
            if proxy.send_event(UserEvent::Push).is_err() {
                break; // Ereignisschleife beendet
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

// ---------------------------------------------------------------------------
// Befehle ausführen
// ---------------------------------------------------------------------------

fn handle_ui(cmd: UiCommand, shared: &SharedRef, proxy: &EventLoopProxy<UserEvent>, webview: &wry::WebView) {
    match cmd {
        UiCommand::Ready | UiCommand::DismissMessage => {
            if matches!(cmd, UiCommand::DismissMessage) {
                let mut s = shared.lock().unwrap();
                s.error = None;
                s.notice = None;
            }
            let _ = proxy.send_event(UserEvent::Push);
        }
        UiCommand::InstallEaApp => spawn_task(shared, proxy, &tr!("EA App wird heruntergeladen …", "Downloading the EA app …"), |cfg, report| {
            let installer = runner::download_ea_installer(|done, total| {
                report(total.map(|t| done as f64 / t as f64));
            })?;
            report(None);
            runner::run_ea_installer(&cfg, &installer)?;
            Ok(tr!("EA App installiert. Starte sie jetzt und melde dich an.", "EA app installed. Start it now and sign in."))
        }),
        UiCommand::StartEaApp => spawn_task(shared, proxy, &tr!("EA App wird gestartet …", "Starting the EA app …"), |cfg, _| {
            runner::start_ea_app(&cfg)?;
            // Warten, bis die lokale API antwortet (max. 3 Minuten, erster Start dauert).
            let api = EaApi::new();
            for _ in 0..90 {
                if api.state() != EaState::NotRunning {
                    return Ok(String::new());
                }
                std::thread::sleep(Duration::from_secs(2));
            }
            let logs = runner::log_dir();
            let logs = logs.display();
            anyhow::bail!("{}", tr!("EA App antwortet nicht. Läuft sie? Logs: {logs}", "The EA app is not responding. Is it running? Logs: {logs}"))
        }),
        UiCommand::StopEaApp => spawn_task(shared, proxy, &tr!("EA App wird beendet …", "Stopping the EA app …"), |cfg, _| {
            runner::stop_prefix(&cfg)?;
            Ok(String::new())
        }),
        UiCommand::OpenEaLibrary => spawn_task(shared, proxy, &tr!("Bibliothek wird geöffnet …", "Opening library …"), |_, _| {
            EaApi::new().open_library()?;
            Ok(String::new())
        }),
        UiCommand::InstallGame { game } => {
            let name = game.name();
            let label = tr!("{name} wird zur Installation eingereiht …", "Queueing {name} for installation …");
            spawn_task(shared, proxy, &label, move |_, _| {
                let api = EaApi::new();
                let offer = api
                    .main_offer(game)?
                    .ok_or_else(|| anyhow::anyhow!("{}", tr!("{name} ist nicht in deiner EA-Bibliothek", "{name} is not in your EA library")))?;
                api.download(&offer.offer_id)?;
                let _ = api.open_library();
                Ok(tr!(
                    "Download von {name} gestartet. Den Fortschritt siehst du hier und in der EA App \
                     (dort ggf. Installationsordner bestätigen).",
                    "Download of {name} started. Progress is shown here and in the EA app \
                     (confirm the install folder there if asked)."
                ))
            });
        }
        UiCommand::OpenInFirefox { game } => {
            let locale = shared.lock().unwrap().cfg.locale.clone();
            let res = runner::open_firefox(&game.server_browser_url(&locale), USER_AGENT).map(|_| {
                tr!(
                    "Battlelog wurde in Firefox geöffnet (eigenes Profil, gibt sich als Windows aus).",
                    "Battlelog opened in Firefox (separate profile, identifies as Windows)."
                )
            });
            set_result(shared, proxy, res);
        }
        UiCommand::OpenServerBrowser { game } => {
            let locale = shared.lock().unwrap().cfg.locale.clone();
            let _ = webview.load_url(&game.server_browser_url(&locale));
        }
        UiCommand::SaveSettings { settings } => {
            let res = (|| -> Result<String> {
                let mut s = shared.lock().unwrap();
                let mut cfg = s.cfg.clone();
                cfg.prefix = settings.prefix.trim().into();
                cfg.proton = settings.proton.trim().into();
                cfg.locale = settings.locale.trim().to_lowercase();
                let restart = cfg.true_players != settings.true_players || cfg.language != settings.language;
                cfg.true_players = settings.true_players;
                cfg.language = settings.language.trim().to_string();
                cfg.check_updates = settings.check_updates;
                cfg.validate()?;
                cfg.save()?;
                s.cfg = cfg;
                Ok(if restart {
                    tr!(
                        "Gespeichert. Einige Änderungen wirken erst nach einem Neustart des Launchers.",
                        "Saved. Some changes take effect after restarting the launcher."
                    )
                } else {
                    tr!("Einstellungen gespeichert.", "Settings saved.")
                })
            })();
            set_result(shared, proxy, res);
        }
        UiCommand::OpenLogs => runner::open_folder(&runner::log_dir()),
        UiCommand::CheckUpdates => {
            let (shared2, proxy2) = (shared.clone(), proxy.clone());
            spawn_task(shared, proxy, &tr!("Suche nach Updates …", "Checking for updates …"), move |_, _| {
                let info = updater::check()?;
                let msg = match &info {
                    Some(_) => String::new(),
                    None => tr!("Der Launcher ist aktuell.", "The launcher is up to date."),
                };
                shared2.lock().unwrap().update = info;
                let _ = proxy2.send_event(UserEvent::Push);
                Ok(msg)
            });
        }
        UiCommand::ApplyUpdate => {
            let shared2 = shared.clone();
            spawn_task(shared, proxy, &tr!("Update wird installiert …", "Installing update …"), move |_, report| {
                updater::apply(&|p| report(Some(p)))?;
                let mut s = shared2.lock().unwrap();
                s.update_ready = true;
                Ok(tr!(
                    "Update installiert. Starte den Launcher neu, um es zu verwenden.",
                    "Update installed. Restart the launcher to use it."
                ))
            });
        }
        UiCommand::RestartLauncher => {
            if shared.lock().unwrap().update_ready {
                updater::restart();
            }
        }
    }
}

fn push_state(shared: &SharedRef, webview: &wry::WebView) {
    if !webview.url().map(|u| is_launcher(&u)).unwrap_or(false) {
        return;
    }
    let state = snapshot(&shared.lock().unwrap());
    if let Ok(json) = serde_json::to_string(&state) {
        // JSON ist ein gültiges JS-Literal; kein String-Zusammenbau mit Benutzereingaben.
        let _ = webview.evaluate_script(&format!("window.bfl && window.bfl.update({json});"));
    }
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn start_update_checker(shared: SharedRef, proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        loop {
            if !shared.lock().unwrap().cfg.check_updates {
                break;
            }
            match updater::check() {
                Ok(info) => {
                    let mut s = shared.lock().unwrap();
                    if !s.update_ready {
                        s.update = info;
                    }
                }
                Err(e) => log::info!("Update-Prüfung: {e:#}"),
            }
            if proxy.send_event(UserEvent::Push).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_secs(30 * 60));
        }
    });
}

fn battlelog_init_script(cfg: &Config) -> String {
    let opts = serde_json::json!({ "truePlayers": cfg.true_players, "lang": i18n::lang().code() });
    format!(
        "(function(){{ if (location.hostname !== {host:?} || window.top !== window) return; \
         window.__BFL_OPTS = Object.freeze({opts}); {BATTLELOG_JS} }})();",
        host = BATTLELOG_HOST,
    )
}

fn run_cli(arg: &str) -> Result<()> {
    let current = format!("{} ({})", updater::VERSION, updater::short(updater::BUILD_COMMIT));
    match arg {
        "--version" | "-V" => println!("bf-launcher {current}"),
        "--check-update" => match updater::check()? {
            Some(u) => {
                let (new, msg) = (updater::short(&u.commit), &u.message);
                println!("{}", tr!("Update verfügbar: {current} → {new} ({msg})", "Update available: {current} → {new} ({msg})"));
            }
            None => println!("{}", tr!("Aktuell: {current}", "Up to date: {current}")),
        },
        "--update" => match updater::check()? {
            Some(u) => {
                updater::apply(&|_| {})?;
                let new = updater::short(&u.commit);
                println!("{}", tr!("Aktualisiert auf {new}.", "Updated to {new}."));
            }
            None => println!("{}", tr!("Aktuell: {current}", "Up to date: {current}")),
        },
        _ => {
            println!(
                "bf-launcher {current}\n\n  --version        {}\n  --check-update   {}\n  --update         {}",
                tr!("Version anzeigen", "Show version"),
                tr!("Auf Updates prüfen", "Check for updates"),
                tr!("Update installieren", "Install update"),
            );
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    updater::remember_exe();
    let cfg = Config::load();
    i18n::init(&cfg.language);

    // Befehlszeile / command line: --version, --check-update, --update
    if let Some(arg) = std::env::args().nth(1) {
        return run_cli(&arg);
    }
    if !Config::path().exists() {
        let _ = cfg.save();
    }
    let debug = std::env::var_os("BFL_DEBUG").is_some();

    let shared: SharedRef = Arc::new(Mutex::new(Shared {
        cfg: cfg.clone(),
        task: None,
        error: None,
        notice: None,
        ea: EaState::NotRunning,
        games: BTreeMap::new(),
        update: None,
        update_ready: false,
    }));

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title("BF Launcher")
        .with_inner_size(LogicalSize::new(1360.0, 860.0))
        .with_min_inner_size(LogicalSize::new(960.0, 640.0))
        .build(&event_loop)?;

    // Eigenes, persistentes Webprofil (Battlelog-Login bleibt erhalten).
    let mut web_context = WebContext::new(Some(config::data_dir().join("webview")));

    let ipc_proxy = proxy.clone();
    let nav_proxy = proxy.clone();
    let win_proxy = proxy.clone();
    let load_proxy = proxy.clone();
    let selftest = std::env::var_os("BFL_SELFTEST").is_some();
    let start_url = match std::env::var("BFL_SELFTEST").ok().and_then(|g| match g.as_str() {
        "bf3" => Some(Game::Bf3),
        "bf4" => Some(Game::Bf4),
        _ => None,
    }) {
        Some(g) => g.server_browser_url(&cfg.locale),
        None => LAUNCHER_URL.to_string(),
    };

    let builder = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_url(&start_url)
        .with_user_agent(USER_AGENT)
        .with_devtools(debug)
        .with_custom_protocol("bfl".into(), |_id, req| serve(req))
        .with_initialization_script(battlelog_init_script(&cfg))
        .with_ipc_handler(move |req: Request<String>| {
            let uri = req.uri().to_string();
            let body = req.body();
            if body.len() > 64 * 1024 {
                return;
            }
            let ev = if is_launcher(&uri) {
                serde_json::from_str::<UiCommand>(body).map(UserEvent::Ui).ok()
            } else if is_battlelog(&uri) {
                serde_json::from_str::<BattlelogCommand>(body).map(UserEvent::Battlelog).ok()
            } else {
                None
            };
            match ev {
                Some(ev) => {
                    let _ = ipc_proxy.send_event(ev);
                }
                None => log::warn!("IPC-Nachricht von {uri} verworfen"),
            }
        })
        .with_navigation_handler(move |url: String| {
            // Erlaubt: eigene Oberfläche und Webinhalte (inkl. eingebetteter Frames
            // wie dem EA-Login). Fremde Schemata (file:, origin2:, link2ea:, …) werden
            // blockiert; EA-Links starten stattdessen die EA App.
            let scheme = url.split(':').next().unwrap_or_default().to_ascii_lowercase();
            match scheme.as_str() {
                "bfl" => is_launcher(&url),
                "https" | "http" | "about" | "data" | "blob" => true,
                "origin" | "origin2" | "link2ea" => {
                    let _ = nav_proxy.send_event(UserEvent::Ui(UiCommand::StartEaApp));
                    false
                }
                _ => {
                    log::warn!("Navigation blockiert: {url}");
                    false
                }
            }
        })
        .with_new_window_req_handler(move |url, _features| {
            // Popups/neue Tabs öffnen im normalen Systembrowser.
            let _ = win_proxy.send_event(UserEvent::External(url));
            NewWindowResponse::Deny
        })
        .with_on_page_load_handler(move |ev, url| {
            if matches!(ev, wry::PageLoadEvent::Finished) {
                let _ = load_proxy.send_event(UserEvent::PageLoaded(url));
            }
        });

    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(window.default_vbox().expect("GTK-Box"))?
    };
    #[cfg(not(target_os = "linux"))]
    let webview = builder.build(&window)?;

    start_poller(shared.clone(), proxy.clone());
    if std::env::var_os("BFL_SELFTEST").is_none() {
        start_update_checker(shared.clone(), proxy.clone());
    }

    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;
        let _ = &web_context; // muss so lange leben wie die WebView
        match event {
            Event::NewEvents(StartCause::Init) => log::info!("BF Launcher gestartet"),
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(UserEvent::Push) => push_state(&shared, &webview),
            Event::UserEvent(UserEvent::Ui(cmd)) => handle_ui(cmd, &shared, &proxy, &webview),
            Event::UserEvent(UserEvent::Battlelog(cmd)) => match cmd {
                BattlelogCommand::BackToLauncher => {
                    let _ = webview.load_url(LAUNCHER_URL);
                }
                BattlelogCommand::OpenInFirefox { game } => {
                    handle_ui(UiCommand::OpenInFirefox { game }, &shared, &proxy, &webview);
                }
                BattlelogCommand::SwitchGame { game } => {
                    let locale = shared.lock().unwrap().cfg.locale.clone();
                    let _ = webview.load_url(&game.server_browser_url(&locale));
                }
                BattlelogCommand::EaRequest { id, method, url, body, timeout } => {
                    use std::sync::atomic::Ordering;
                    let p = proxy.clone();
                    if BRIDGE_INFLIGHT.fetch_add(1, Ordering::SeqCst) >= BRIDGE_MAX {
                        BRIDGE_INFLIGHT.fetch_sub(1, Ordering::SeqCst);
                        let resp = ea_api::BridgeResponse { status: 0, body: String::new(), content_type: String::new() };
                        let _ = p.send_event(UserEvent::EaReply { id, resp });
                    } else {
                        std::thread::spawn(move || {
                            let resp = ea_api::bridge_request(&method, &url, body.as_deref(), timeout.unwrap_or(30_000));
                            BRIDGE_INFLIGHT.fetch_sub(1, Ordering::SeqCst);
                            let _ = p.send_event(UserEvent::EaReply { id, resp });
                        });
                    }
                }
            },
            Event::UserEvent(UserEvent::EaReply { id, resp }) => {
                if webview.url().map(|u| is_battlelog(&u)).unwrap_or(false) {
                    let js = format!(
                        "window.__bflEaReply && window.__bflEaReply({id}, {}, {}, {});",
                        resp.status,
                        serde_json::to_string(&resp.body).unwrap_or_else(|_| "\"\"".into()),
                        serde_json::to_string(&resp.content_type).unwrap_or_else(|_| "\"\"".into()),
                    );
                    let _ = webview.evaluate_script(&js);
                }
            }
            Event::UserEvent(UserEvent::External(url)) => runner::open_external(&url),
            Event::UserEvent(UserEvent::SelftestCollect) => {
                let _ = webview.evaluate_script_with_callback("window.__bflSelftest || (document.querySelectorAll('[hidden]').forEach(e => e.remove()), document.body.innerText)", |res| {
                    println!("SELFTEST {res}");
                });
            }
            Event::UserEvent(UserEvent::PageLoaded(url)) => {
                log::debug!("Seite geladen: {url}");
                if is_launcher(&url) {
                    push_state(&shared, &webview);
                    if selftest {
                        let p = proxy.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_secs(6));
                            let _ = p.send_event(UserEvent::SelftestCollect);
                        });
                    }
                } else if selftest && is_battlelog(&url) {
                    let _ = webview.evaluate_script(SELFTEST_JS);
                    let p = proxy.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_secs(12));
                        let _ = p.send_event(UserEvent::SelftestCollect);
                    });
                }
            }
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins() {
        assert!(is_launcher("bfl://localhost/"));
        assert!(!is_launcher("bfl://evil/"));
        assert!(is_battlelog("https://battlelog.battlefield.com/bf4/servers/"));
        assert!(!is_battlelog("https://battlelog.battlefield.com.evil.net/"));
        assert!(!is_battlelog("https://evil.net/?battlelog.battlefield.com"));
        assert!(!is_battlelog("https://battlelog.battlefield.com@evil.net/"));
        assert!(!is_battlelog("http://battlelog.battlefield.com/"));
    }

    #[test]
    fn commands_parse() {
        assert!(matches!(
            serde_json::from_str::<UiCommand>(r#"{"cmd":"install_game","game":"bf4"}"#),
            Ok(UiCommand::InstallGame { game: Game::Bf4 })
        ));
        assert!(serde_json::from_str::<UiCommand>(r#"{"cmd":"install_game","game":"bf5"}"#).is_err());
        assert!(serde_json::from_str::<BattlelogCommand>(r#"{"cmd":"install_ea_app"}"#).is_err());
    }
}
