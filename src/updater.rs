//! Selbstaktualisierung über GitHub-Releases.
//!
//! Bei jedem Push auf `main` baut GitHub Actions den Launcher und veröffentlicht
//! ihn im Release mit dem Tag `latest` (siehe .github/workflows/release.yml),
//! zusammen mit `version.json` (Commit) und einer SHA-256-Prüfsumme.
//! Der Launcher vergleicht den Commit, aus dem er gebaut wurde, mit dem des
//! Releases und kann sich bei einer Abweichung selbst ersetzen.

use crate::tr;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

pub const REPO: &str = match option_env!("BFL_REPO") {
    Some(r) => r,
    None => "xNeo92x/bf-launcher",
};
pub const BUILD_COMMIT: &str = env!("BFL_COMMIT");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const ASSET: &str = "bf-launcher-x86_64-linux";
const MAX_BINARY: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpdateInfo {
    pub commit: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub message: String,
}

pub fn short(commit: &str) -> &str {
    &commit[..commit.len().min(7)]
}

/// Pfad der laufenden Programmdatei – beim Start festgehalten, weil
/// /proc/self/exe nach dem Ersetzen auf "(deleted)" zeigt.
static EXE: OnceLock<PathBuf> = OnceLock::new();

pub fn remember_exe() {
    if let Ok(p) = std::env::current_exe() {
        let _ = EXE.set(p);
    }
}

fn exe() -> Result<PathBuf> {
    EXE.get()
        .cloned()
        .or_else(|| std::env::current_exe().ok())
        .context("current_exe")
}

fn asset_url(name: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/latest/{name}")
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(timeout))
        .user_agent(concat!("bf-launcher/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

/// Fragt das neueste Release ab. `Some` = es gibt einen anderen Stand als den laufenden.
pub fn check() -> Result<Option<UpdateInfo>> {
    let mut resp = agent(Duration::from_secs(20))
        .get(asset_url("version.json"))
        .call()
        .with_context(|| tr!("Update-Prüfung fehlgeschlagen", "Update check failed"))?;
    let text = resp.body_mut().with_config().limit(64 * 1024).read_to_string()?;
    let info: UpdateInfo = serde_json::from_str(&text)?;
    anyhow::ensure!(
        info.commit.len() >= 7 && info.commit.chars().all(|c| c.is_ascii_hexdigit()),
        "invalid version.json"
    );
    Ok((info.commit != BUILD_COMMIT).then_some(info))
}

fn download(url: &str, limit: u64, mut out: impl Write, progress: &dyn Fn(f64)) -> Result<u64> {
    let mut resp = agent(Duration::from_secs(600)).get(url).call()?;
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if total.is_some_and(|t| t > limit) {
        bail!("download too large");
    }
    let mut reader = resp.body_mut().with_config().limit(limit).reader();
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        done += n as u64;
        if let Some(t) = total {
            progress(done as f64 / t as f64);
        }
    }
    if total.is_some_and(|t| t != done) {
        bail!("download incomplete");
    }
    Ok(done)
}

/// Lädt die neue Version, prüft sie und ersetzt die laufende Programmdatei.
pub fn apply(progress: &dyn Fn(f64)) -> Result<()> {
    let exe = exe()?;
    let dir = exe.parent().context("exe dir")?;
    let tmp = dir.join(".bf-launcher.update");

    // Prüfsumme zuerst (klein), dann die Programmdatei.
    let mut sum = Vec::new();
    download(&asset_url(&format!("{ASSET}.sha256")), 4096, &mut sum, &|_| {})?;
    let expected = String::from_utf8_lossy(&sum)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    anyhow::ensure!(expected.len() == 64, "invalid checksum file");

    let res = (|| -> Result<()> {
        let mut file = File::create(&tmp).with_context(|| {
            let d = dir.display();
            tr!(
                "Kein Schreibzugriff auf {d} – bitte den Launcher neu mit install.sh installieren",
                "No write access to {d} – please reinstall the launcher with install.sh"
            )
        })?;
        download(&asset_url(ASSET), MAX_BINARY, &mut file, progress)?;
        file.sync_all()?;
        drop(file);

        let mut hasher = Sha256::new();
        let mut f = File::open(&tmp)?;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if actual != expected {
            bail!("{}", tr!("Prüfsumme stimmt nicht – Update verworfen", "Checksum mismatch – update discarded"));
        }
        let mut magic = [0u8; 4];
        File::open(&tmp)?.read_exact(&mut magic)?;
        anyhow::ensure!(&magic == b"\x7fELF", "not an ELF binary");

        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
        // Atomar ersetzen; der laufende Prozess behält die alte Datei.
        fs::rename(&tmp, &exe)?;
        Ok(())
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

/// Startet die (neue) Programmdatei und beendet den aktuellen Prozess.
pub fn restart() -> ! {
    if let Ok(exe) = exe() {
        let _ = std::process::Command::new(exe).spawn();
    }
    std::process::exit(0);
}
