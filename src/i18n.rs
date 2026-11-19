//! Sprache der Oberfläche: Deutsch, wenn das System auf Deutsch eingestellt ist,
//! sonst Englisch. Kann in der Konfiguration überschrieben werden.
//!
//! UI language: German if the system locale is German, otherwise English.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    De,
    En,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::De => "de",
            Lang::En => "en",
        }
    }
}

static LANG: OnceLock<Lang> = OnceLock::new();

/// Ermittelt die Systemsprache aus den üblichen POSIX-Variablen.
pub fn detect_system() -> Lang {
    detect_from(|k| std::env::var(k).ok())
}

fn detect_from(get: impl Fn(&str) -> Option<String>) -> Lang {
    // Reihenfolge wie bei gettext: LANGUAGE (Prioritätsliste), LC_ALL, LC_MESSAGES, LANG.
    let first = ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| get(k))
        .map(|v| v.split(':').next().unwrap_or_default().to_string())
        .find(|v| !v.is_empty() && v != "C" && v != "POSIX" && !v.starts_with("C."));
    match first {
        Some(v) if v.to_ascii_lowercase().starts_with("de") => Lang::De,
        _ => Lang::En,
    }
}

/// Legt die Sprache fest ("auto", "de" oder "en"). Nur der erste Aufruf zählt.
pub fn init(setting: &str) -> Lang {
    let lang = match setting {
        "de" => Lang::De,
        "en" => Lang::En,
        _ => detect_system(),
    };
    *LANG.get_or_init(|| lang)
}

pub fn lang() -> Lang {
    *LANG.get_or_init(detect_system)
}

pub fn is_de() -> bool {
    lang() == Lang::De
}

/// `tr!("Deutsch {x}", "English {x}")` – formatiert in der aktiven Sprache.
#[macro_export]
macro_rules! tr {
    ($de:literal, $en:literal $(,)?) => {
        if $crate::i18n::is_de() { format!($de) } else { format!($en) }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn detection() {
        assert_eq!(detect_from(env(&[("LANG", "de_DE.UTF-8")])), Lang::De);
        assert_eq!(detect_from(env(&[("LANG", "de_AT.UTF-8")])), Lang::De);
        assert_eq!(detect_from(env(&[("LANG", "en_US.UTF-8")])), Lang::En);
        assert_eq!(detect_from(env(&[("LANG", "fr_FR.UTF-8")])), Lang::En);
        assert_eq!(detect_from(env(&[("LANG", "C.UTF-8")])), Lang::En);
        assert_eq!(detect_from(env(&[])), Lang::En);
        assert_eq!(detect_from(env(&[("LC_ALL", "en_GB.UTF-8"), ("LANG", "de_DE.UTF-8")])), Lang::En);
        assert_eq!(detect_from(env(&[("LANGUAGE", "de:en"), ("LANG", "en_US.UTF-8")])), Lang::De);
    }
}
