/*
 * BF Launcher – Ergänzungen für Battlelog (läuft nur auf battlelog.battlefield.com,
 * nur im Hauptframe).
 *
 * 1. Echte Spielerzahlen im Serverbrowser von BF3 und BF4.
 *    Battlelog zeigt in der Serverliste die vom Server gemeldete Zahl an, die
 *    viele Server künstlich aufblähen. Wie das BBLog-Plugin "True Players"
 *    (github.com/Dendari92/bblog-true-players-bf3) fragen wir pro Server den
 *    Battlelog-Endpunkt /<spiel>/servers/getPlayersOnServer/pc/<guid>/ ab und
 *    zählen die tatsächlich verbundenen Spieler. Eigene Implementierung,
 *    zusätzlich mit BF4-Unterstützung.
 * 2. Eine kleine Leiste zum Zurückkehren in den Launcher und zum Spielwechsel.
 */
(function () {
  "use strict";

  const OPTS = window.__BFL_OPTS || { truePlayers: true, lang: "en" };
  const L = OPTS.lang === "de"
    ? { back: "← Launcher", onlyReal: "nur echte Zahl", hideEmpty: "leere ausblenden",
        unavailable: "Echte Spielerzahl nicht abrufbar",
        tip: (c, s) => `Echte Spieler: ${c} (Battlelog zeigt ${s})` }
    : { back: "← Launcher", onlyReal: "real count only", hideEmpty: "hide empty",
        unavailable: "Real player count unavailable",
        tip: (c, s) => `Real players: ${c} (Battlelog shows ${s})` };
  const STORE_KEY = "bfl.settings";
  const CACHE_MS = 20000;
  const MAX_PARALLEL = 6;

  const game = (location.pathname.match(/^\/(bf3|bf4)\b/) || [])[1] || null;

  // ---- Einstellungen (pro Browserprofil gespeichert) -----------------------
  const settings = Object.assign({ onlyReal: false, hideEmpty: false }, (() => {
    try { return JSON.parse(localStorage.getItem(STORE_KEY)) || {}; } catch (_) { return {}; }
  })());
  function saveSettings() {
    try { localStorage.setItem(STORE_KEY, JSON.stringify(settings)); } catch (_) {}
    applyBodyClasses();
  }
  function applyBodyClasses() {
    const root = document.documentElement;
    root.classList.toggle("bfl-only-real", !!settings.onlyReal);
    root.classList.toggle("bfl-hide-empty", !!settings.hideEmpty);
  }

  function ipc(msg) {
    try { window.ipc.postMessage(JSON.stringify(msg)); } catch (e) { console.warn("[BFL]", e); }
  }

  // ---- Styles ---------------------------------------------------------------
  function injectStyle() {
    if (document.getElementById("bfl-style")) return;
    const st = document.createElement("style");
    st.id = "bfl-style";
    st.textContent = `
      .bfl-real { font-weight: 700; margin: 0 4px; padding: 0 5px; border-radius: 3px;
                  font-size: 0.95em; white-space: nowrap; }
      .bfl-real.bfl-good { color: #fff; background: #2e8b57; }
      .bfl-real.bfl-mid  { color: #111; background: #f0b429; }
      .bfl-real.bfl-bad  { color: #fff; background: #c0392b; }
      .bfl-real.bfl-wait { color: #aaa; background: transparent; }
      html.bfl-only-real .bfl-fake { display: none !important; }
      html.bfl-hide-empty .bfl-empty { display: none !important; }
    `;
    (document.head || document.documentElement).appendChild(st);
  }

  // ---- Echte Spielerzahlen --------------------------------------------------
  const cache = new Map();   // guid -> {t, count}
  const pending = new Map(); // guid -> Promise
  const queue = [];
  let running = 0;

  function fetchCount(guid) {
    const hit = cache.get(guid);
    if (hit && Date.now() - hit.t < CACHE_MS) return Promise.resolve(hit.count);
    if (pending.has(guid)) return pending.get(guid);
    const p = new Promise((resolve) => queue.push({ guid, resolve }));
    pending.set(guid, p);
    pump();
    return p;
  }

  function pump() {
    while (running < MAX_PARALLEL && queue.length) {
      const { guid, resolve } = queue.shift();
      running++;
      fetch(`/${game}/servers/getPlayersOnServer/pc/${encodeURIComponent(guid)}/`, {
        credentials: "same-origin",
        headers: { "X-Requested-With": "XMLHttpRequest", "Accept": "application/json" },
      })
        .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
        .then((data) => {
          const players = Array.isArray(data && data.players) ? data.players : [];
          const count = players.length;
          cache.set(guid, { t: Date.now(), count });
          resolve(count);
        })
        .catch(() => resolve(null))
        .finally(() => { running--; pending.delete(guid); pump(); });
    }
  }

  function classify(real, shown) {
    if (real === null) return "bfl-wait";
    if (real === 0 && shown > 0) return "bfl-bad";
    if (shown > 0 && real / shown < 0.6) return "bfl-mid";
    return "bfl-good";
  }

  // Adapter je Spiel: liefert {row, guid, cell, shown}
  const adapters = {
    bf3: {
      rows: () => document.querySelectorAll(".serverguide-bodycells[guid]"),
      guid: (row) => row.getAttribute("guid"),
      prepare(row) {
        const cell = row.querySelector(".serverguide-cell-players");
        if (!cell) return null;
        const done = cell.querySelector(".bfl-processed");
        if (done) {
          const fake = done.querySelector(".bfl-fake");
          return { shown: parseInt(fake && fake.textContent, 10) || 0, real: done.querySelector(".bfl-real") };
        }
        const span = cell.querySelector("span");
        if (!span) return null;
        const m = (span.textContent || "").match(/(\d+)\s*\/\s*(\d+)/);
        if (!m) return null;
        // "38 / 48" -> <span class=bfl-fake>38</span><span class=bfl-real>[6]</span> / 48
        span.textContent = "";
        const fake = document.createElement("span");
        fake.className = "bfl-fake";
        fake.textContent = m[1];
        const real = document.createElement("span");
        real.className = "bfl-real bfl-wait";
        real.textContent = "…";
        span.append(fake, real, document.createTextNode(" / " + m[2]));
        span.classList.add("bfl-processed");
        return { shown: parseInt(m[1], 10), real };
      },
    },
    bf4: {
      rows: () => document.querySelectorAll("tr.server-row[data-guid]"),
      guid: (row) => row.getAttribute("data-guid"),
      prepare(row) {
        const cell = row.querySelector("td.players");
        const occ = cell && cell.querySelector(".occupied");
        if (!occ) return null;
        occ.classList.add("bfl-fake");
        let real = cell.querySelector(".bfl-real");
        if (!real) {
          real = document.createElement("span");
          real.className = "bfl-real bfl-wait";
          real.textContent = "…";
          occ.after(real);
        }
        return { shown: parseInt(occ.textContent, 10) || 0, real };
      },
    },
  };

  function processRows() {
    const ad = adapters[game];
    if (!ad) return;
    ad.rows().forEach((row) => {
      const guid = ad.guid(row);
      if (!guid || !/^[0-9a-f-]{36}$/i.test(guid)) return;
      const info = ad.prepare(row);
      if (!info) return;
      // Bereits für diese GUID und Anzeige erledigt?
      const key = guid + ":" + info.shown;
      if (row.dataset.bflKey === key && !info.real.classList.contains("bfl-wait")) return;
      row.dataset.bflKey = key;
      fetchCount(guid).then((count) => {
        if (!row.isConnected) return;
        info.real.className = "bfl-real " + classify(count, info.shown);
        info.real.textContent = count === null ? "?" : "[" + count + "]";
        info.real.title = count === null ? L.unavailable : L.tip(count, info.shown);
        row.classList.toggle("bfl-empty", count === 0);
      });
    });
  }

  let scheduled = false;
  function schedule() {
    if (scheduled) return;
    scheduled = true;
    setTimeout(() => { scheduled = false; processRows(); }, 150);
  }

  // ---- Leiste ---------------------------------------------------------------
  function buildBar() {
    if (document.getElementById("bfl-bar")) return;
    const host = document.createElement("div");
    host.id = "bfl-bar";
    host.style.cssText = "position:fixed;left:16px;bottom:16px;z-index:2147483647;";
    const root = host.attachShadow({ mode: "closed" });
    const style = document.createElement("style");
    style.textContent = `
      .bar { display:flex; gap:6px; align-items:center; padding:6px; border-radius:12px;
             background:rgba(14,17,22,.92); border:1px solid rgba(255,255,255,.12);
             box-shadow:0 8px 24px rgba(0,0,0,.45); font:13px/1.2 system-ui,sans-serif; color:#e8eaed; }
      button { all:unset; cursor:pointer; padding:7px 11px; border-radius:8px; color:#e8eaed; }
      button:hover { background:rgba(255,255,255,.1); }
      button.on { background:#f28c28; color:#111; font-weight:600; }
      label { display:flex; gap:6px; align-items:center; padding:0 8px; cursor:pointer; user-select:none; }
      .sep { width:1px; align-self:stretch; background:rgba(255,255,255,.15); }
    `;
    const bar = document.createElement("div");
    bar.className = "bar";

    const mkBtn = (text, onClick, on) => {
      const b = document.createElement("button");
      b.textContent = text;
      if (on) b.className = "on";
      b.addEventListener("click", onClick);
      return b;
    };
    const mkToggle = (text, key) => {
      const l = document.createElement("label");
      const c = document.createElement("input");
      c.type = "checkbox";
      c.checked = !!settings[key];
      c.addEventListener("change", () => { settings[key] = c.checked; saveSettings(); });
      l.append(c, document.createTextNode(text));
      return l;
    };

    bar.append(
      mkBtn(L.back, () => ipc({ cmd: "back_to_launcher" })),
      document.createElement("div"),
      mkBtn("BF3", () => ipc({ cmd: "switch_game", game: "bf3" }), game === "bf3"),
      mkBtn("BF4", () => ipc({ cmd: "switch_game", game: "bf4" }), game === "bf4"),
    );
    bar.children[1].className = "sep";
    if (OPTS.truePlayers && game) {
      const sep = document.createElement("div");
      sep.className = "sep";
      bar.append(sep, mkToggle(L.onlyReal, "onlyReal"), mkToggle(L.hideEmpty, "hideEmpty"));
    }
    root.append(style, bar);
    document.body.appendChild(host);
  }

  // ---- Brücke zur EA App ----------------------------------------------------
  // Battlelog ruft die EA App per jQuery.ajax unter http://127.0.0.1:3215 auf
  // (und das laufende Spiel unter :4219). WebKitGTK blockiert das als gemischten
  // Inhalt. Ein jQuery-Transport leitet genau diese Anfragen per IPC an den
  // Launcher weiter, der sie prüft und ausführt.
  const LOCAL_RE = /^http:\/\/(127\.0\.0\.1|localhost):(3215|4219)(\/|\?|$)/;
  const waiting = new Map();
  let nextId = 1;

  Object.defineProperty(window, "__bflEaReply", {
    value: function (id, status, body, contentType) {
      const cb = waiting.get(id);
      if (!cb) return;
      waiting.delete(id);
      cb(status, body, contentType);
    },
    writable: false,
    configurable: false,
  });

  function installBridge($) {
    if (!$ || !$.ajaxTransport || $.__bflBridge) return !!($ && $.__bflBridge);
    $.__bflBridge = true;
    $.ajaxTransport("+*", function (options) {
      if (!LOCAL_RE.test(options.url || "")) return undefined;
      let id = 0;
      return {
        send: function (_headers, complete) {
          id = nextId++;
          let timer = null;
          waiting.set(id, function (status, body, contentType) {
            if (timer) clearTimeout(timer);
            if (status === 0) return complete(0, "error", {});
            complete(status, status >= 200 && status < 300 ? "success" : "error",
              { text: body }, contentType ? "Content-Type: " + contentType : "");
          });
          const timeout = options.timeout > 0 ? options.timeout : 30000;
          // Lokale Zeitüberschreitung als Absicherung (Antwort kommt sonst nie).
          timer = setTimeout(() => window.__bflEaReply(id, 0, "", ""), timeout + 2000);
          ipc({
            cmd: "ea_request",
            id,
            method: (options.type || "GET").toUpperCase(),
            url: options.url,
            body: typeof options.data === "string" ? options.data : null,
            timeout,
          });
        },
        abort: function () { waiting.delete(id); },
      };
    });
    return true;
  }

  // jQuery wird von Battlelog geladen; so früh wie möglich einhängen.
  (function waitForJQuery(tries) {
    if (installBridge(window.jQuery)) return;
    if (tries > 0) setTimeout(() => waitForJQuery(tries - 1), 50);
  })(400);

  function start() {
    applyBodyClasses();
    buildBar();
    if (OPTS.truePlayers && game) {
      injectStyle();
      processRows();
      new MutationObserver(schedule).observe(document.body, { childList: true, subtree: true });
      // Battlelog aktualisiert Listen teils ohne DOM-Austausch: regelmäßig nachziehen.
      setInterval(() => { cache.clear(); document.querySelectorAll("[data-bfl-key]").forEach((r) => delete r.dataset.bflKey); schedule(); }, 30000);
    }
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start, { once: true });
  } else {
    start();
  }
})();
