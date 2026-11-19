//! Unterstützte Spiele.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Game {
    Bf3,
    Bf4,
}

impl Game {
    pub const ALL: [Game; 2] = [Game::Bf3, Game::Bf4];

    pub fn name(self) -> &'static str {
        match self {
            Game::Bf3 => "Battlefield 3",
            Game::Bf4 => "Battlefield 4",
        }
    }

    /// Kürzel in Battlelog-URLs.
    pub fn slug(self) -> &'static str {
        match self {
            Game::Bf3 => "bf3",
            Game::Bf4 => "bf4",
        }
    }

    /// "masterTitleId", unter dem die EA App das Spiel führt
    /// (identisch zu den Werten, die Battlelog selbst verwendet).
    pub fn master_title_id(self) -> &'static str {
        match self {
            Game::Bf3 => "50182",
            Game::Bf4 => "76889",
        }
    }

    /// Angebote, die Battlelog nicht als Hauptspiel zählt (z. B. BF4 CTE).
    pub fn ignored_offers(self) -> &'static [&'static str] {
        match self {
            Game::Bf3 => &[],
            Game::Bf4 => &["Origin.OFR.50.0000427"],
        }
    }

    pub fn server_browser_url(self, locale: &str) -> String {
        format!("https://battlelog.battlefield.com/{}/{}/servers/", self.slug(), locale)
    }
}
