//! Startet Windows-Programme (EA App, Installer) über umu-launcher + Proton.
//!
//! Es wird nie eine Shell verwendet: Alle Programme werden direkt mit
//! Argumentlisten gestartet, Benutzereingaben landen nie in Kommandozeilen.

use crate::config::{Config, cache_dir, data_dir};
use crate::tr;
use anyhow::{Context, Result, bail};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Offizieller Download der EA App (vom EA-CDN, HTTPS).
pub const EA_INSTALLER_URL: &str =
    "https://origin-a.akamaihd.net/EA-Desktop-Client-Download/installer-releases/EAappInstaller.exe";

const EA_APP_REL: &str = "drive_c/Program Files/Electronic Arts/EA Desktop/EA Desktop/EADesktop.exe";

pub fn umu_available() -> bool {
    find_in_path("umu-run").is_some()
}

fn find_in_path(bin: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .map(|d| Path::new(d).join(bin))
        .find(|p| p.is_file())
}

pub fn ea_app_exe(cfg: &Config) -> PathBuf {
    cfg.prefix.join(EA_APP_REL)
}

pub fn ea_app_installed(cfg: &Config) -> bool {
    ea_app_exe(cfg).is_file()
}

fn log_file(name: &str) -> Result<File> {
    let dir = data_dir().join("logs");
    fs::create_dir_all(&dir)?;
    Ok(File::create(dir.join(name))?)
}

pub fn log_dir() -> PathBuf {
    data_dir().join("logs")
}

/// Baut einen umu-run-Aufruf für das konfigurierte Präfix.
fn umu(cfg: &Config, exe: &Path, args: &[&str], log: &str) -> Result<Command> {
    let umu = find_in_path("umu-run").with_context(|| {
        tr!(
            "umu-run nicht gefunden. Installiere es mit: sudo pacman -S umu-launcher",
            "umu-run not found. Install it with: sudo pacman -S umu-launcher"
        )
    })?;
    fs::create_dir_all(&cfg.prefix)
        .with_context(|| tr!("Präfix-Ordner anlegen", "Creating prefix directory"))?;
    let mut cmd = Command::new(umu);
    cmd.arg(exe)
        .args(args)
        .env("WINEPREFIX", &cfg.prefix)
        .env("GAMEID", "umu-default")
        .env("STORE", "ea")
        .env("PROTONPATH", &cfg.proton)
        .stdin(Stdio::null());
    for (k, v) in &cfg.extra_env {
        cmd.env(k, v);
    }
    let out = log_file(log)?;
    let err = out.try_clone()?;
    cmd.stdout(out).stderr(err);
    Ok(cmd)
}

/// Lädt den offiziellen EA-App-Installer herunter und gibt den Pfad zurück.
pub fn download_ea_installer(progress: impl Fn(u64, Option<u64>)) -> Result<PathBuf> {
    let dir = cache_dir();
    fs::create_dir_all(&dir)?;
    let target = dir.join("EAappInstaller.exe");
    let part = dir.join("EAappInstaller.exe.part");

    let agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .https_only(true)
        .build()
        .new_agent();
    let mut resp = agent.get(EA_INSTALLER_URL).call().with_context(|| tr!("Download der EA App", "Downloading the EA app"))?;
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let mut reader = resp
        .body_mut()
        .with_config()
        .limit(512 * 1024 * 1024)
        .reader();
    let mut file = File::create(&part)?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        progress(done, total);
    }
    file.sync_all()?;
    drop(file);

    // Plausibilitätsprüfung: vollständige Datei und Windows-PE-Header.
    if let Some(t) = total
        && t != done
    {
        bail!("{}", tr!("Download unvollständig ({done} von {t} Bytes)", "Download incomplete ({done} of {t} bytes)"));
    }
    let mut head = [0u8; 2];
    File::open(&part)?.read_exact(&mut head)?;
    if &head != b"MZ" || done < 1024 * 1024 {
        let _ = fs::remove_file(&part);
        bail!("{}", tr!("Heruntergeladene Datei ist kein gültiger Installer", "Downloaded file is not a valid installer"));
    }
    fs::rename(&part, &target)?;
    Ok(target)
}

/// Führt den Installer im Präfix aus und wartet auf sein Ende.
pub fn run_ea_installer(cfg: &Config, installer: &Path) -> Result<()> {
    let status = umu(cfg, installer, &[], "ea-installer.log")?
        .status()
        .with_context(|| tr!("Installer starten", "Starting installer"))?;
    if !ea_app_installed(cfg) {
        let log = log_dir().join("ea-installer.log");
        let log = log.display();
        bail!(
            "{}",
            tr!(
                "EA App wurde nicht gefunden (Installer-Exit: {status}). Details: {log}",
                "EA app not found (installer exit: {status}). Details: {log}"
            )
        );
    }
    Ok(())
}

/// Startet die EA App im Hintergrund (non-blocking).
pub fn start_ea_app(cfg: &Config) -> Result<()> {
    let exe = ea_app_exe(cfg);
    if !exe.is_file() {
        bail!("{}", tr!("EA App ist noch nicht installiert", "The EA app is not installed yet"));
    }
    let mut child = umu(cfg, &exe, &[], "ea-app.log")?
        .spawn()
        .with_context(|| tr!("EA App starten", "Starting the EA app"))?;
    // Prozess im Hintergrund einsammeln, damit kein Zombie bleibt.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Beendet alle Prozesse im Präfix (EA App + Spiel).
pub fn stop_prefix(cfg: &Config) -> Result<()> {
    let proton = Path::new(&cfg.proton);
    let wineserver = proton.join("files/bin/wineserver");
    let bin = if wineserver.is_file() {
        wineserver
    } else {
        find_in_path("wineserver").with_context(|| tr!("wineserver nicht gefunden", "wineserver not found"))?
    };
    Command::new(bin)
        .arg("-k")
        .env("WINEPREFIX", &cfg.prefix)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(())
}

/// Öffnet eine http(s)-URL im Standardbrowser des Systems.
pub fn open_external(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        log::warn!("Externe URL abgelehnt: {url}");
        return;
    }
    if let Err(e) = Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|mut c| std::thread::spawn(move || c.wait()))
    {
        log::warn!("xdg-open fehlgeschlagen: {e}");
    }
}

pub fn open_folder(path: &Path) {
    let _ = fs::create_dir_all(path);
    if let Ok(mut c) = Command::new("xdg-open").arg(path).stdin(Stdio::null()).spawn() {
        std::thread::spawn(move || c.wait());
    }
}
