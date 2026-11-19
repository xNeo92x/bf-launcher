//! Persistente Einstellungen (~/.config/bf-launcher/config.toml).

use crate::tr;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Wine/Proton-Präfix, in dem die EA App und die Spiele installiert werden.
    pub prefix: PathBuf,
    /// Proton-Version für umu-run: Pfad zu einem Proton-Ordner oder ein
    /// umu-Schlüsselwort wie "GE-Proton" (lädt automatisch die neueste Version).
    pub proton: String,
    /// Battlelog-Sprache (de, en, fr, ...).
    pub locale: String,
    /// Echte Spielerzahlen im Serverbrowser anzeigen.
    pub true_players: bool,
    /// Zusätzliche Umgebungsvariablen für die EA App (z. B. DXVK_HUD=1).
    pub extra_env: Vec<(String, String)>,
    /// Sprache des Launchers: "auto" (Systemsprache), "de" oder "en".
    pub language: String,
    /// Automatisch auf GitHub nach Updates suchen.
    pub check_updates: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            prefix: data_dir().join("prefix"),
            proton: detect_proton().unwrap_or_else(|| "GE-Proton".into()),
            locale: if crate::i18n::detect_system() == crate::i18n::Lang::De { "de" } else { "en" }.into(),
            true_players: true,
            extra_env: Vec::new(),
            language: "auto".into(),
            check_updates: true,
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("bf-launcher")
            .join("config.toml")
    }

    pub fn load() -> Self {
        let path = Self::path();
        match fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                log::warn!("Konfiguration {path:?} fehlerhaft ({e}), nutze Standardwerte");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, toml::to_string_pretty(self)?)?;
        fs::rename(&tmp, &path).with_context(|| tr!("Konfiguration speichern", "Saving configuration"))?;
        Ok(())
    }

    /// Validiert vom Benutzer gesetzte Werte, bevor sie übernommen werden.
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.prefix.is_absolute(),
            "{}",
            tr!("Präfix muss ein absoluter Pfad sein", "Prefix must be an absolute path")
        );
        anyhow::ensure!(
            self.locale.len() == 2 && self.locale.chars().all(|c| c.is_ascii_lowercase()),
            "{}",
            tr!("Ungültige Battlelog-Sprache", "Invalid Battlelog language")
        );
        anyhow::ensure!(
            !self.proton.is_empty() && !self.proton.contains('\0'),
            "{}",
            tr!("Ungültige Proton-Angabe", "Invalid Proton setting")
        );
        anyhow::ensure!(
            matches!(self.language.as_str(), "auto" | "de" | "en"),
            "{}",
            tr!("Ungültige Sprache", "Invalid language")
        );
        for (k, _) in &self.extra_env {
            anyhow::ensure!(
                !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "{}",
                tr!("Ungültiger Variablenname: {k}", "Invalid variable name: {k}")
            );
        }
        Ok(())
    }
}

pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bf-launcher")
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("bf-launcher")
}

/// Sucht die neueste installierte GE-Proton-Version (Steam compatibilitytools.d).
pub fn detect_proton() -> Option<String> {
    let home = dirs::home_dir()?;
    let roots = [
        home.join(".local/share/Steam/compatibilitytools.d"),
        home.join(".steam/root/compatibilitytools.d"),
        PathBuf::from("/usr/share/steam/compatibilitytools.d"),
    ];
    let mut found: Vec<PathBuf> = roots
        .iter()
        .filter_map(|r| fs::read_dir(r).ok())
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_proton_dir(p))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("GE-Proton"))
        })
        .collect();
    found.sort_by_key(|p| version_key(p));
    found.pop().map(|p| p.to_string_lossy().into_owned())
}

fn is_proton_dir(p: &Path) -> bool {
    p.join("proton").is_file()
}

/// "GE-Proton11-5-x86_64" -> [11, 5]
fn version_key(p: &Path) -> Vec<u32> {
    p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|s| s.parse().ok())
        .filter(|&n| n != 86 && n != 64)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_sorting() {
        assert!(version_key(Path::new("GE-Proton11-5-x86_64")) > version_key(Path::new("GE-Proton11-3")));
        assert!(version_key(Path::new("GE-Proton10-20")) < version_key(Path::new("GE-Proton11-1")));
    }

    #[test]
    fn validation() {
        let mut c = Config::default();
        assert!(c.validate().is_ok());
        c.locale = "../x".into();
        assert!(c.validate().is_err());
        c.locale = "de".into();
        c.extra_env = vec![("BAD;NAME".into(), "1".into())];
        assert!(c.validate().is_err());
    }
}
