# BF Launcher

[English](README.md) · **Deutsch**

Ein Linux-Launcher in Rust für **Battlefield 3** und **Battlefield 4**, entwickelt für CachyOS und Arch.

- **Einrichtung und Anmeldung:** Der Launcher installiert die offizielle EA App in einem eigenen Proton-Präfix (über `umu-launcher`) und startet sie. Du meldest dich im Fenster der EA App an. Der Launcher sieht und speichert keine Zugangsdaten.
- **Spieleauswahl:** Für BF3 und BF4 wird jeweils der Status aus deiner EA-Bibliothek angezeigt. Mit **Herunterladen & installieren** startet der Download in der EA App, und der Fortschritt erscheint direkt im Launcher.
- **Serverbrowser:** Ist ein Spiel installiert, öffnest du mit **Serverbrowser** das echte Battlelog des Spiels (`/bf3/<sprache>/servers/` bzw. `/bf4/<sprache>/servers/`) im Launcher-Fenster. Dort kannst du wie gewohnt Servern beitreten. Das Spiel startet dann über die EA App.
- **Echte Spielerzahlen:** In der Serverliste erscheint hinter der von Battlelog gemeldeten Zahl die tatsächliche Zahl verbundener Spieler, zum Beispiel `38 [6] / 48`. Die Farbe zeigt, wie stark die Zahl geschönt ist. Die Leiste unten links hat die Optionen **nur echte Zahl** und **leere ausblenden**. Die Idee stammt vom BBLog-Plugin [bblog-true-players-bf3](https://github.com/Dendari92/bblog-true-players-bf3). Der Launcher nutzt eine eigene Umsetzung, die auch BF4 unterstützt.
- **Sprache:** Ist dein System auf Deutsch eingestellt, erscheint der Launcher auf Deutsch, sonst auf Englisch. In den Einstellungen lässt sich das ändern.
- **Automatische Updates:** Jede Änderung an diesem Repository wird automatisch gebaut. Der Launcher erkennt die neue Version und kann sich mit einem Klick selbst aktualisieren.

## Installation

### Fertiges Programm

```bash
sudo pacman -S --needed webkit2gtk-4.1 umu-launcher
mkdir -p ~/.local/bin
curl -L -o ~/.local/bin/bf-launcher https://github.com/xNeo92x/bf-launcher/releases/download/latest/bf-launcher-x86_64-linux
chmod +x ~/.local/bin/bf-launcher
```

### Aus dem Quellcode bauen

```bash
sudo pacman -S --needed rust webkit2gtk-4.1 umu-launcher
git clone https://github.com/xNeo92x/bf-launcher.git
cd bf-launcher
./install.sh
```

`install.sh` trägt **BF Launcher** zusätzlich ins Startmenü ein. Du startest ihn von dort oder mit `bf-launcher`.

## Erster Start

1. **EA App installieren:** Der Launcher lädt den offiziellen Installer von EA herunter und führt ihn im Präfix aus. Folge dem Installer-Fenster.
2. **EA App starten** und dort anmelden. Sobald die Anmeldung erkannt ist, erscheint die Spieleauswahl.
3. Bei BF3 oder BF4 auf **Herunterladen & installieren** klicken. Je nach EA-App-Version musst du den Installationsordner in der EA App noch bestätigen.
4. Nach der Installation auf **Serverbrowser** klicken. Beim ersten Mal meldest du dich auf der EA-Anmeldeseite bei Battlelog an. Die Sitzung bleibt danach gespeichert.

## Updates

Bei jedem Push auf `main` baut GitHub Actions den Launcher. Danach veröffentlicht es das Programm, eine SHA-256-Prüfsumme und `version.json` (den Commit) im Release [`latest`](https://github.com/xNeo92x/bf-launcher/releases/tag/latest).

Der Launcher vergleicht den Commit, aus dem er gebaut wurde, mit diesem Release. Er prüft beim Start und danach alle 30 Minuten, oder sofort über **Einstellungen → Jetzt nach Updates suchen**. Weichen die Commits ab, bietet ein Hinweis **Jetzt aktualisieren** an. Der Launcher lädt dann die neue Version, prüft Prüfsumme und ELF-Header und ersetzt seine Programmdatei atomar. Nach **Neu starten** läuft die neue Version.

Die automatische Prüfung lässt sich in den Einstellungen abschalten. Die Programmdatei muss für deinen Benutzer beschreibbar sein, zum Beispiel in `~/.local/bin`.

## Funktionsweise

```
┌──────────── BF Launcher (Rust: tao + wry/WebKitGTK) ─────────────┐
│ bfl://  Launcher-UI (eingebettet, strikte CSP, kein Netzwerk)    │
│ https://battlelog.battlefield.com  + eingebettetes Skript:       │
│     • echte Spielerzahlen (getPlayersOnServer)                   │
│     • Leiste "← Launcher / BF3 / BF4"                            │
│     • Brücke: jQuery-Anfragen an 127.0.0.1:3215/4219 ──IPC──┐    │
│ Rust-Backend ◄──────────────────────────────────────────────┘    │
│     • prüft Ziel und Pfad (Allowlist) und leitet weiter          │
│     • fragt Status und Downloads bei der EA App ab               │
│     • prüft auf Updates über GitHub-Releases                     │
└───────────────┬──────────────────────────────────────────────────┘
                │ http://127.0.0.1:3215 (lokale API der EA App)
        EA App in Proton (umu-run, eigenes Präfix) ──► bf3.exe / bf4.exe
```

Battlelog startet Spiele über eine lokale HTTP-API der EA App (`/game/launch`, `/game/status` …). Unter Proton ist diese API auch auf dem Linux-Host erreichbar. WebKitGTK blockiert aber Anfragen einer HTTPS-Seite an `http://127.0.0.1` als gemischte Inhalte. Deshalb hängt der Launcher einen jQuery-Transport in Battlelog ein. Dieser reicht genau diese Anfragen an das Rust-Backend weiter, und das Backend führt sie aus.

## Sicherheit

- **Keine Zugangsdaten im Launcher:** Die Anmeldung läuft nur in der EA App und auf EAs eigener Anmeldeseite.
- **Getrennte IPC-Rechte:** Befehle werden nach Herkunft getrennt.
  - Die Launcher-Oberfläche (`bfl://localhost`) darf Launcher-Aktionen auslösen.
  - Battlelog darf nur drei Dinge: zurück zum Launcher, Spiel wechseln und Anfragen an die EA App.
  - EA-App-Anfragen von Battlelog laufen über eine strenge Allowlist: nur `127.0.0.1:3215` mit den Pfaden `/ping`, `/game`, `/library`, `/store` und `/version` sowie `127.0.0.1:4219/`. Höchstens 24 Anfragen laufen gleichzeitig, und Antworten sind auf 2 MB begrenzt.
  - Alle Nachrichten werden mit `serde` streng typisiert geparst (`deny_unknown_fields`).
- **Strikte CSP für die Launcher-Oberfläche:** keine externen Inhalte, kein Inline-Code.
- **Navigation:** `file:` und andere fremde Schemata werden blockiert. Popups öffnen sich im Systembrowser, aber nur `http(s)`.
- **Prozesse:** Programme werden ohne Shell gestartet, mit festen Argumentlisten. Einstellungen werden vor dem Speichern validiert.
- **Downloads:** Der EA-App-Installer kommt nur über HTTPS vom offiziellen EA-CDN. Updates kommen nur über HTTPS von GitHub-Releases und werden gegen ihre SHA-256-Prüfsumme geprüft.

## Dateien

| Pfad | Inhalt |
|---|---|
| `~/.config/bf-launcher/config.toml` | Einstellungen (Präfix, Proton, Sprachen, Updates) |
| `~/.local/share/bf-launcher/prefix` | Proton-Präfix mit EA App und Spielen |
| `~/.local/share/bf-launcher/webview` | Webprofil (Battlelog-Login) |
| `~/.local/share/bf-launcher/logs` | Logs der EA App und des Installers |

Als Proton wird automatisch die neueste installierte **GE-Proton**-Version verwendet. Alternativ trägst du `GE-Proton` ein, dann lädt umu sie selbst herunter.

## Diagnose

```bash
RUST_LOG=bf_launcher=debug bf-launcher          # ausführliche Logs
BFL_DEBUG=1 bf-launcher                         # WebKit-Entwicklerwerkzeuge (Rechtsklick → Untersuchen)
BFL_SELFTEST=bf4 bf-launcher                    # prüft Spielerzahlen und EA-App-Brücke auf Battlelog
bf-launcher --version | --check-update | --update   # Version / Update über die Befehlszeile
```

## Hinweise und Grenzen

- **Spielbarkeit unter Proton:** Ob BF3/BF4 laufen, hängt von Proton und dem Anti-Cheat ab (PunkBuster/FairFight bzw. EA Javelin). Das kann EA jederzeit ändern. Der Launcher kann das nicht umgehen.
- **Browser-Kennung:** Battlelog erlaubt den Spielstart nur aus Windows-Browsern (Prüfung auf dem Server, Dialog „Unable to launch the game from this device“). Unter Proton läuft das Spiel ohnehin als Windows-Programm, deshalb meldet sich der eingebettete Browser als Firefox unter Windows (32-Bit).
- **Battlelog-Limit:** Battlelog sperrt eine IP zeitweise (HTTP 429), wenn sie zu viele Anfragen sendet. Solange die Sperre gilt, scheitert auch Battlelog selbst, zum Beispiel beim Regionsfilter. Der Launcher fragt echte Spielerzahlen deshalb nur für sichtbare Server ab, höchstens etwa 15 pro Minute, speichert sie 2 Minuten und pausiert bei HTTP 429 automatisch.
- **Inoffizielle Endpunkte:** Battlelog-Endpunkte wie `getPlayersOnServer` und die lokale API der EA App sind nicht offiziell dokumentiert und können sich ändern.
- **Lizenz der Vorlage:** Das BBLog-Plugin von Dendari92 hat keine Lizenz. Deshalb enthält der Launcher keinen Code daraus, nur eine eigene Umsetzung derselben Idee.

## Lizenz

[MIT](LICENSE)
