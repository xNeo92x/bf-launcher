#!/usr/bin/env bash
# Baut den BF Launcher und installiert ihn für den aktuellen Benutzer.
# Builds BF Launcher and installs it for the current user.
set -euo pipefail
cd "$(dirname "$0")"

de() { [[ "${LANGUAGE:-${LC_ALL:-${LC_MESSAGES:-${LANG:-}}}}" == de* ]]; }
msg() { if de; then echo "$1"; else echo "$2"; fi; }

missing=()
for pkg in webkit2gtk-4.1 umu-launcher; do
  pacman -Q "$pkg" &>/dev/null || missing+=("$pkg")
done
if ((${#missing[@]})); then
  msg "Fehlende Pakete: ${missing[*]}" "Missing packages: ${missing[*]}"
  msg "Installieren mit: sudo pacman -S --needed ${missing[*]}" "Install with: sudo pacman -S --needed ${missing[*]}"
  exit 1
fi

cargo build --release --locked 2>/dev/null || cargo build --release

install -Dm755 target/release/bf-launcher "$HOME/.local/bin/bf-launcher"
install -Dm644 bf-launcher.desktop "$HOME/.local/share/applications/bf-launcher.desktop"
sed -i "s|^Exec=.*|Exec=$HOME/.local/bin/bf-launcher|" "$HOME/.local/share/applications/bf-launcher.desktop"
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true

msg "Installiert: $HOME/.local/bin/bf-launcher (Startmenü: „BF Launcher“)" \
    "Installed: $HOME/.local/bin/bf-launcher (application menu: \"BF Launcher\")"
