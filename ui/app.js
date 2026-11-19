"use strict";

(function () {
  const $ = (sel, root = document) => root.querySelector(sel);
  const send = (msg) => window.ipc.postMessage(JSON.stringify(msg));

  // ---- Übersetzungen / translations ----------------------------------------
  const I18N = {
    de: {
      btnLibrary: "EA-Bibliothek",
      btnLibraryTitle: "Bibliothek in der EA App öffnen",
      btnStop: "EA App beenden",
      btnStopTitle: "EA App und Spiele im Präfix beenden",
      btnSettings: "Einstellungen",
      close: "Schließen",
      setupTitle: "Einrichtung",
      setupLead: "Battlefield 3 und 4 laufen über die EA App, die der Launcher in einem eigenen Proton-Präfix betreibt. Deine Anmeldung erfolgt ausschließlich im Fenster der EA App – der Launcher sieht und speichert keine Zugangsdaten.",
      umuHint: "Wird zum Starten von Windows-Programmen mit Proton benötigt.",
      eaInstallTitle: "EA App installieren",
      eaInstallHint: "Lädt den offiziellen Installer von EA herunter und führt ihn im Präfix aus.",
      loginTitle: "EA App starten & anmelden",
      loginHint: "Melde dich im EA-App-Fenster an. Danach erscheint hier deine Spieleauswahl. Der erste Start kann ein bis zwei Minuten dauern.",
      startEa: "EA App starten",
      startEaWaiting: "EA App läuft – bitte dort anmelden",
      libraryTitle: "Deine Spiele",
      serverBrowser: "Serverbrowser",
      install: "Herunterladen & installieren",
      footnote: "Im Serverbrowser (Battlelog) meldest du dich beim ersten Mal mit deinem EA-Konto auf der EA-Anmeldeseite an. Hinter jeder Spielerzahl steht in Klammern die <b>echte</b> Anzahl verbundener Spieler. „Beitreten“ startet das Spiel über die EA App.",
      setPrefix: "Wine/Proton-Präfix",
      setProton: "Proton (Pfad oder „GE-Proton“ für automatischen Download)",
      setLanguage: "Sprache des Launchers",
      langAuto: "Automatisch (Systemsprache)",
      setLocale: "Battlelog-Sprache",
      setTruePlayers: "Echte Spielerzahlen im Serverbrowser",
      setUpdates: "Automatisch nach Updates suchen",
      version: "Version",
      checkNow: "Jetzt nach Updates suchen",
      openLogs: "Log-Ordner öffnen",
      cancel: "Abbrechen",
      save: "Speichern",
      updateAvailable: (c) => `Ein Update ist verfügbar (${c}).`,
      updateNow: "Jetzt aktualisieren",
      updateReady: "Update installiert – bitte neu starten.",
      restartNow: "Neu starten",
      status: {
        unknown: "Status unbekannt",
        not_owned: "Nicht in deiner EA-Bibliothek",
        not_installed: "Nicht installiert",
        downloading: "Wird heruntergeladen / installiert",
        installed: "Installiert – bereit",
      },
      ea: {
        not_running: "EA App: nicht gestartet",
        logged_out: "EA App: nicht angemeldet",
        logged_in: "EA App: angemeldet",
      },
    },
    en: {
      btnLibrary: "EA library",
      btnLibraryTitle: "Open the library in the EA app",
      btnStop: "Stop EA app",
      btnStopTitle: "Stop the EA app and games in the prefix",
      btnSettings: "Settings",
      close: "Close",
      setupTitle: "Setup",
      setupLead: "Battlefield 3 and 4 run through the EA app, which the launcher runs in its own Proton prefix. You sign in only in the EA app window – the launcher never sees or stores your credentials.",
      umuHint: "Required to run Windows programs with Proton.",
      eaInstallTitle: "Install EA app",
      eaInstallHint: "Downloads the official installer from EA and runs it in the prefix.",
      loginTitle: "Start the EA app & sign in",
      loginHint: "Sign in in the EA app window. Your games will then appear here. The first start can take a minute or two.",
      startEa: "Start EA app",
      startEaWaiting: "EA app is running – please sign in there",
      libraryTitle: "Your games",
      serverBrowser: "Server browser",
      install: "Download & install",
      footnote: "In the server browser (Battlelog), sign in with your EA account on EA's sign-in page the first time. The number in brackets after each player count is the <b>real</b> number of connected players. “Join” starts the game through the EA app.",
      setPrefix: "Wine/Proton prefix",
      setProton: "Proton (path, or “GE-Proton” for automatic download)",
      setLanguage: "Launcher language",
      langAuto: "Automatic (system language)",
      setLocale: "Battlelog language",
      setTruePlayers: "Real player counts in the server browser",
      setUpdates: "Check for updates automatically",
      version: "Version",
      checkNow: "Check for updates now",
      openLogs: "Open log folder",
      cancel: "Cancel",
      save: "Save",
      updateAvailable: (c) => `An update is available (${c}).`,
      updateNow: "Update now",
      updateReady: "Update installed – please restart.",
      restartNow: "Restart",
      status: {
        unknown: "Status unknown",
        not_owned: "Not in your EA library",
        not_installed: "Not installed",
        downloading: "Downloading / installing",
        installed: "Installed – ready",
      },
      ea: {
        not_running: "EA app: not running",
        logged_out: "EA app: signed out",
        logged_in: "EA app: signed in",
      },
    },
  };
  const T = I18N[document.documentElement.lang] || I18N.en;

  function translate() {
    document.querySelectorAll("[data-i18n]").forEach((el) => (el.textContent = T[el.dataset.i18n]));
    // Nur statische, eigene Texte – keine Benutzereingaben.
    document.querySelectorAll("[data-i18n-html]").forEach((el) => (el.innerHTML = T[el.dataset.i18nHtml]));
    document.querySelectorAll("[data-i18n-title]").forEach((el) => (el.title = T[el.dataset.i18nTitle]));
    document.querySelectorAll("[data-i18n-aria-label]").forEach((el) => el.setAttribute("aria-label", T[el.dataset.i18nAriaLabel]));
    $("#ea-status .txt").textContent = T.ea.not_running;
  }

  let state = null;

  function setStep(li, done, active) {
    li.classList.toggle("done", done);
    li.classList.toggle("active", active);
    $(".badge", li).textContent = done ? "✓" : "";
    li.querySelectorAll("button").forEach((b) => (b.disabled = !active || !!(state && state.task)));
  }

  function render(s) {
    state = s;
    const busy = !!s.task;

    // Kopfzeile
    const ea = $("#ea-status");
    ea.dataset.state = s.ea;
    $(".txt", ea).textContent = T.ea[s.ea] || s.ea;
    $("#btn-library").disabled = busy || s.ea !== "logged_in";
    $("#btn-stop").disabled = busy || s.ea === "not_running";

    // Update
    const upd = $("#update");
    upd.hidden = !(s.update || s.update_ready);
    if (s.update_ready) {
      $(".msg", upd).textContent = T.updateReady;
      $("#btn-update").textContent = T.restartNow;
      $("#btn-update").dataset.action = "restart";
    } else if (s.update) {
      $(".msg", upd).textContent = T.updateAvailable(s.update.commit.slice(0, 7));
      $("#btn-update").textContent = T.updateNow;
      $("#btn-update").dataset.action = "apply";
    }
    $("#btn-update").disabled = busy;
    $("#version").textContent = s.version;

    // Meldungen
    const banner = $("#banner");
    const msg = s.error || s.notice;
    banner.hidden = !msg;
    banner.classList.toggle("error", !!s.error);
    $(".msg", banner).textContent = msg || "";

    // Laufende Aufgabe
    const task = $("#task");
    task.hidden = !s.task;
    if (s.task) {
      $(".label", task).textContent = s.task.label;
      const fill = $(".fill", task);
      const p = s.task.progress;
      task.classList.toggle("indeterminate", p === null || p === undefined);
      fill.style.width = p == null ? "" : Math.round(p * 100) + "%";
    }

    // Einrichtung vs. Bibliothek
    const ready = s.ea === "logged_in";
    $("#setup").hidden = ready;
    $("#library").hidden = !ready;

    setStep($("#step-umu"), s.umu, !s.umu);
    setStep($("#step-ea"), s.ea_installed, s.umu && !s.ea_installed);
    setStep($("#step-login"), ready, s.umu && s.ea_installed && !ready);
    $("#btn-start-ea").textContent = s.ea === "logged_out" ? T.startEaWaiting : T.startEa;
    if (s.ea === "logged_out") $("#btn-start-ea").disabled = true;

    // Spielkarten
    document.querySelectorAll(".card").forEach((card) => {
      const g = s.games[card.dataset.game] || { status: "unknown", progress: 0 };
      card.dataset.status = g.status;
      let text = T.status[g.status] || g.status;
      if (g.status === "downloading") {
        text += " – " + Math.round((g.progress || 0) * 100) + " %";
        if (g.state) text += " (" + g.state.toLowerCase().replace(/_/g, " ") + ")";
      }
      $(".status", card).textContent = text;
      const prog = $(".progress", card);
      prog.hidden = g.status !== "downloading";
      $(".fill", prog).style.width = Math.round((g.progress || 0) * 100) + "%";
      $(".play", card).hidden = g.status !== "installed";
      const install = $(".install", card);
      install.hidden = g.status !== "not_installed";
      install.disabled = busy;
    });
  }

  function openSettings() {
    if (!state) return;
    const f = $("#settings-form");
    const st = state.settings;
    f.prefix.value = st.prefix;
    f.proton.value = st.proton;
    f.locale.value = st.locale;
    f.language.value = st.language;
    f.true_players.checked = st.true_players;
    f.check_updates.checked = st.check_updates;
    $("#settings").showModal();
  }

  function init() {
    translate();
    window.bfl = Object.freeze({ update: render });

    $("#btn-install-ea").addEventListener("click", () => send({ cmd: "install_ea_app" }));
    $("#btn-start-ea").addEventListener("click", () => send({ cmd: "start_ea_app" }));
    $("#btn-stop").addEventListener("click", () => send({ cmd: "stop_ea_app" }));
    $("#btn-library").addEventListener("click", () => send({ cmd: "open_ea_library" }));
    $("#btn-settings").addEventListener("click", openSettings);
    $("#banner-close").addEventListener("click", () => send({ cmd: "dismiss_message" }));
    $("#btn-logs").addEventListener("click", () => send({ cmd: "open_logs" }));
    $("#settings-cancel").addEventListener("click", () => $("#settings").close());
    $("#btn-check-updates").addEventListener("click", () => {
      $("#settings").close();
      send({ cmd: "check_updates" });
    });
    $("#btn-update").addEventListener("click", (e) => {
      send({ cmd: e.currentTarget.dataset.action === "restart" ? "restart_launcher" : "apply_update" });
    });

    $("#settings-form").addEventListener("submit", (e) => {
      const f = e.target;
      send({
        cmd: "save_settings",
        settings: {
          prefix: f.prefix.value,
          proton: f.proton.value,
          locale: f.locale.value,
          true_players: f.true_players.checked,
          language: f.language.value,
          check_updates: f.check_updates.checked,
        },
      });
    });

    document.querySelectorAll(".card").forEach((card) => {
      const game = card.dataset.game;
      $(".play", card).addEventListener("click", () => send({ cmd: "open_server_browser", game }));
      $(".install", card).addEventListener("click", () => send({ cmd: "install_game", game }));
    });

    send({ cmd: "ready" });
  }

  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", init);
  else init();
})();
