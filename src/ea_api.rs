//! Client für die lokale HTTP-API der EA App (http://127.0.0.1:3215).
//!
//! Dieselbe Schnittstelle nutzt Battlelog, um Spiele zu starten. Läuft die EA App
//! unter Proton, bindet sie den Port auf dem Loopback-Interface des Linux-Hosts,
//! sodass sowohl dieser Launcher als auch die eingebettete Battlelog-Seite sie
//! direkt erreichen. Der Launcher sieht dabei niemals Zugangsdaten: Die Anmeldung
//! passiert ausschließlich im Fenster der EA App.

use crate::games::Game;
use crate::tr;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use ureq::Agent;

const BASE: &str = "http://127.0.0.1:3215";
// Die EA App akzeptiert Anfragen von Battlelog; wir identifizieren uns genauso.
const ORIGIN: &str = "https://battlelog.battlefield.com";

#[derive(Clone)]
pub struct EaApi {
    agent: Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EaState {
    /// API nicht erreichbar – EA App läuft nicht (oder startet noch).
    NotRunning,
    /// EA App läuft, aber niemand ist angemeldet.
    LoggedOut,
    LoggedIn,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    #[serde(default)]
    pub offer_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub play_state: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default, deserialize_with = "lenient_f64")]
    pub progress: f64,
    #[serde(default)]
    pub update_available: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct StatusResponse {
    #[serde(default)]
    offers: Vec<Offer>,
}

fn lenient_f64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    })
}

impl Offer {
    pub fn state(&self) -> String {
        self.state.clone().unwrap_or_default().to_uppercase()
    }
    pub fn is_installed(&self) -> bool {
        self.state() == "INSTALLED"
            && self.play_state.as_deref().map(str::to_uppercase).as_deref() != Some("NOT_PLAYABLE")
    }
    pub fn is_base_game(&self) -> bool {
        self.content_type.as_deref() == Some("BASE_GAME")
    }
}

impl EaApi {
    pub fn new() -> Self {
        let agent = Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(5)))
            .http_status_as_error(false)
            .max_redirects(0)
            .build()
            .new_agent();
        Self { agent }
    }

    fn get(&self, path: &str) -> Result<(u16, String)> {
        let mut resp = self
            .agent
            .get(format!("{BASE}{path}"))
            .header("Origin", ORIGIN)
            .header("Referer", format!("{ORIGIN}/"))
            .call()?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().with_config().limit(1 << 20).read_to_string()?;
        Ok((status, body))
    }

    fn post(&self, path: &str) -> Result<(u16, String)> {
        let mut resp = self
            .agent
            .post(format!("{BASE}{path}"))
            .header("Origin", ORIGIN)
            .header("Referer", format!("{ORIGIN}/"))
            .send_empty()?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().with_config().limit(1 << 20).read_to_string()?;
        Ok((status, body))
    }

    pub fn state(&self) -> EaState {
        match self.get("/ping") {
            Ok((200, body)) if body.contains("pong") => {}
            _ => return EaState::NotRunning,
        }
        // Wie Battlelog: Abfrage eines nicht existierenden Spiels liefert 403,
        // wenn niemand angemeldet ist, sonst einen anderen Fehlercode.
        match self.get("/game/status?masterTitleId=000000") {
            Ok((403, _)) => EaState::LoggedOut,
            Ok(_) => EaState::LoggedIn,
            Err(_) => EaState::NotRunning,
        }
    }

    pub fn offers(&self, game: Game) -> Result<Vec<Offer>> {
        let (status, body) = self.get(&format!(
            "/game/status?masterTitleId={}",
            game.master_title_id()
        ))?;
        if status == 404 {
            return Ok(Vec::new()); // nicht in der Bibliothek
        }
        if status != 200 {
            bail!("{}", tr!("EA App antwortete mit HTTP {status}", "EA app responded with HTTP {status}"));
        }
        Ok(serde_json::from_str::<StatusResponse>(&body)?.offers)
    }

    /// Wählt das Hauptspiel-Angebot (bevorzugt ein installiertes), wie Battlelog.
    pub fn main_offer(&self, game: Game) -> Result<Option<Offer>> {
        let offers = self.offers(game)?;
        let mut candidates = offers
            .into_iter()
            .filter(|o| o.is_base_game() && !game.ignored_offers().contains(&o.offer_id.as_str()));
        let mut first = None;
        for o in candidates.by_ref() {
            if o.is_installed() {
                return Ok(Some(o));
            }
            first.get_or_insert(o);
        }
        Ok(first)
    }

    /// Startet Download + Installation in der EA App.
    pub fn download(&self, offer_id: &str) -> Result<()> {
        anyhow::ensure!(
            offer_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '-' | '_')),
            "{}",
            tr!("Ungültige Angebots-ID", "Invalid offer ID")
        );
        let (status, body) = self.post(&format!(
            "/game/download?refresh=true&frontOfQueue=true&offerId={}",
            urlencoding::encode(offer_id)
        ))?;
        if !(200..300).contains(&status) {
            let body = body.trim();
            bail!("{}", tr!("Download konnte nicht gestartet werden (HTTP {status}): {body}", "Could not start download (HTTP {status}): {body}"));
        }
        Ok(())
    }

    /// Öffnet (und aktualisiert) die Bibliothek in der EA App.
    pub fn open_library(&self) -> Result<()> {
        self.post("/library/open?refresh=true")?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Brücke für Battlelog
// ---------------------------------------------------------------------------
//
// WebKitGTK blockiert Anfragen einer https-Seite an http://127.0.0.1 (gemischte
// Inhalte). Battlelog spricht die EA App aber genau so an. Das eingebettete
// Skript leitet diese Anfragen deshalb per IPC an den Launcher, der sie nach
// strenger Prüfung (nur bekannte Ziele und Pfade) selbst ausführt.

/// Ergebnis einer weitergeleiteten Anfrage (Status 0 = Verbindungsfehler).
pub struct BridgeResponse {
    pub status: u16,
    pub body: String,
    pub content_type: String,
}

/// Prüft eine von Battlelog angefragte URL und gibt die normalisierte URL zurück.
pub fn bridge_target(url: &str) -> Option<String> {
    if url.len() > 4096 || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let rest = url
        .strip_prefix("http://127.0.0.1:")
        .or_else(|| url.strip_prefix("http://localhost:"))?;
    let (port, path) = match rest.find(['/', '?']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let path = if path.starts_with('?') { format!("/{path}") } else { path.to_string() };
    if path.contains("..") || path.contains("//") || path.contains('#') || path.contains('\\') {
        return None;
    }
    let first = path[1..].split(['/', '?']).next().unwrap_or_default();
    let ok = match port {
        // Lokale API der EA App
        "3215" => matches!(first, "ping" | "game" | "library" | "store" | "version"),
        // Ereigniskanal des laufenden Spiels (Battlelog-"Snowroller")
        "4219" => first.is_empty(),
        _ => false,
    };
    ok.then(|| format!("http://127.0.0.1:{port}{path}"))
}

pub fn bridge_request(method: &str, url: &str, body: Option<&str>, timeout_ms: u64) -> BridgeResponse {
    let fail = || BridgeResponse { status: 0, body: String::new(), content_type: String::new() };
    let Some(target) = bridge_target(url) else {
        log::warn!("Battlelog-Anfrage an {url} abgelehnt");
        return fail();
    };
    let timeout = Duration::from_millis(timeout_ms.clamp(500, 60_000));
    let agent: Agent = Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(timeout))
        .build()
        .new_agent();
    let res = match method {
        "GET" => agent.get(&target).header("Origin", ORIGIN).header("Referer", format!("{ORIGIN}/")).call(),
        "POST" => {
            let req = agent
                .post(&target)
                .header("Origin", ORIGIN)
                .header("Referer", format!("{ORIGIN}/"));
            match body {
                Some(b) if !b.is_empty() => req
                    .header("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8")
                    .send(b.as_bytes()),
                _ => req.send_empty(),
            }
        }
        _ => return fail(),
    };
    match res {
        Ok(mut r) => {
            let status = r.status().as_u16();
            let content_type = r
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            let body = r.body_mut().with_config().limit(2 << 20).read_to_string().unwrap_or_default();
            BridgeResponse { status, body, content_type }
        }
        Err(e) => {
            log::debug!("Brücke {target}: {e}");
            fail()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_allowlist() {
        assert_eq!(bridge_target("http://127.0.0.1:3215/ping").as_deref(), Some("http://127.0.0.1:3215/ping"));
        assert!(bridge_target("http://127.0.0.1:3215/game/launch?offerIds=OFB-EAST%3A1&cmdParams=-x").is_some());
        assert_eq!(bridge_target("http://127.0.0.1:4219").as_deref(), Some("http://127.0.0.1:4219/"));
        assert!(bridge_target("http://127.0.0.1:3215/../etc").is_none());
        assert!(bridge_target("http://127.0.0.1:3215/admin").is_none());
        assert!(bridge_target("http://127.0.0.1:22/").is_none());
        assert!(bridge_target("http://127.0.0.1:3215@evil.com/ping").is_none());
        assert!(bridge_target("http://evil.com/ping").is_none());
        assert!(bridge_target("http://127.0.0.1:3215/ping\r\nX: y").is_none());
        assert!(bridge_target("http://127.0.0.1:4219/foo").is_none());
    }

    #[test]
    fn parse_offer() {
        let json = r#"{"offers":[{"offerId":"OFB-EAST:109552316","state":"INSTALLED",
            "playState":"READY_TO_PLAY","contentType":"BASE_GAME","progress":"1.0"}]}"#;
        let r: StatusResponse = serde_json::from_str(json).unwrap();
        assert!(r.offers[0].is_installed());
        assert!(r.offers[0].is_base_game());
        assert_eq!(r.offers[0].progress, 1.0);
    }
}
