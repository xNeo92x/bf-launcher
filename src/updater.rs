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
const BUILD_TIME: &str = env!("BFL_COMMIT_TIME");

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
    /// Commit-Zeitpunkt (Unix-Zeit)
    #[serde(default)]
    pub timestamp: Option<u64>,
}

/// Nur ein neuerer Stand gilt als Update (Schutz vor Downgrades durch veraltete Daten).
fn is_newer(remote: &UpdateInfo, local_commit: &str, local_time: u64) -> bool {
    if remote.commit == local_commit {
        return false;
    }
    match remote.timestamp {
        Some(t) if local_time > 0 => t > local_time,
        _ => true,
    }
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

// Hinweis: Die öffentlichen Links github.com/…/releases/download/latest/… werden von
// GitHub mehrere Minuten zwischengespeichert. Deshalb fragen wir das Release über die
// API ab und laden jede Datei über ihre eindeutige Asset-ID (immer der aktuelle Stand).

#[derive(Deserialize)]
struct Release {
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    id: u64,
    name: String,
}

/// Asset-IDs des Releases "latest" (eine API-Anfrage).
fn release_assets() -> Result<Vec<Asset>> {
    let mut resp = agent(Duration::from_secs(20))
        .get(format!("https://api.github.com/repos/{REPO}/releases/tags/latest"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .with_context(|| tr!("Update-Prüfung fehlgeschlagen", "Update check failed"))?;
    let text = resp.body_mut().with_config().limit(1 << 20).read_to_string()?;
    Ok(serde_json::from_str::<Release>(&text)?.assets)
}

fn asset_url(assets: &[Asset], name: &str) -> Result<String> {
    let a = assets
        .iter()
        .find(|a| a.name == name)
        .with_context(|| format!("release asset {name} missing"))?;
    Ok(format!("https://api.github.com/repos/{REPO}/releases/assets/{}", a.id))
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
    let assets = release_assets()?;
    let mut buf = Vec::new();
    download(&asset_url(&assets, "version.json")?, 64 * 1024, &mut buf, &|_| {})?;
    let info: UpdateInfo = serde_json::from_slice(&buf)?;
    anyhow::ensure!(
        info.commit.len() >= 7 && info.commit.chars().all(|c| c.is_ascii_hexdigit()),
        "invalid version.json"
    );
    let local_time = BUILD_TIME.parse().unwrap_or(0);
    Ok(is_newer(&info, BUILD_COMMIT, local_time).then_some(info))
}

fn download(url: &str, limit: u64, mut out: impl Write, progress: &dyn Fn(f64)) -> Result<u64> {
    let mut resp = agent(Duration::from_secs(600))
        .get(url)
        .header("Accept", "application/octet-stream")
        .call()?;
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
    let assets = release_assets()?;
    let mut sum = Vec::new();
    download(&asset_url(&assets, &format!("{ASSET}.sha256"))?, 4096, &mut sum, &|_| {})?;
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
        download(&asset_url(&assets, ASSET)?, MAX_BINARY, &mut file, progress)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn info(commit: &str, ts: Option<u64>) -> UpdateInfo {
        UpdateInfo { commit: commit.into(), version: String::new(), date: String::new(), message: String::new(), timestamp: ts }
    }

    #[test]
    fn newer_only() {
        assert!(!is_newer(&info("aaaaaaa", Some(200)), "aaaaaaa", 100));
        assert!(is_newer(&info("bbbbbbb", Some(200)), "aaaaaaa", 100));
        assert!(!is_newer(&info("bbbbbbb", Some(50)), "aaaaaaa", 100)); // älter -> kein Downgrade
        assert!(is_newer(&info("bbbbbbb", None), "aaaaaaa", 100)); // alte version.json
        assert!(is_newer(&info("bbbbbbb", Some(50)), "unknown", 0)); // lokal unbekannt
    }
}
