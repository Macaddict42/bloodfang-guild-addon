/* global __TAURI__ */
const $ = (id) => document.getElementById(id);
const invoke = (cmd, args) => {
  if (window.__TAURI__?.core?.invoke) return window.__TAURI__.core.invoke(cmd, args);
  if (window.__TAURI_INTERNALS__?.invoke) return window.__TAURI_INTERNALS__.invoke(cmd, args);
  return Promise.reject(new Error("Tauri bridge missing (browser preview)"));
};
const listen = async (event, handler) => {
  const l = window.__TAURI__?.event?.listen;
  if (l) return l(event, handler);
};

function show(id) {
  $("screen-setup").classList.toggle("hidden", id !== "setup");
  $("screen-main").classList.toggle("hidden", id !== "main");
}

function fillSetup(snap) {
  $("site-url").value = snap.prefs.siteUrl || snap.defaults.siteUrl || "";
  $("game-folder").value = snap.prefs.gameFolder || "";
  $("start-login").checked = !!snap.prefs.startAtLogin;
  const box = $("candidates");
  box.innerHTML = "";
  if (snap.candidates?.length) {
    box.hidden = false;
    snap.candidates.forEach((p) => {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = p;
      b.addEventListener("click", () => { $("game-folder").value = p; });
      box.appendChild(b);
    });
  } else box.hidden = true;
}

function fillMain(snap) {
  $("last-upload").textContent = snap.prefs.lastUploadAt || "—";
  $("last-status").textContent = snap.prefs.lastStatus || "—";
  $("addon-ver").textContent = snap.addonInstalled || snap.prefs.lastAddonVersion || "—";
}

async function refresh() {
  const snap = await invoke("get_snapshot");
  fillSetup(snap);
  fillMain(snap);
  if (snap.prefs.setupDone && snap.hasToken) {
    show("main");
    try { await invoke("start_watch"); } catch (_) {}
    try { await refreshAddon(); } catch (_) {}
  } else show("setup");
  return snap;
}

async function refreshAddon() {
  const c = await invoke("check_addon");
  $("addon-installed").textContent = c.installed || "not installed";
  $("addon-latest").textContent = c.latest || "—";
  $("addon-ver").textContent = c.installed || "—";
  $("btn-update").disabled = !c.updateAvailable;
  $("addon-warn").hidden = !c.gameRunning;
  return c;
}

$("btn-save").addEventListener("click", async () => {
  $("setup-err").hidden = true;
  try {
    const snap = await invoke("save_setup", {
      input: {
        siteUrl: $("site-url").value.trim(),
        token: $("token").value.trim(),
        gameFolder: $("game-folder").value.trim(),
        startAtLogin: $("start-login").checked,
      },
    });
    $("token").value = "";
    fillMain(snap);
    show("main");
    await invoke("start_watch");
    await refreshAddon();
  } catch (e) {
    $("setup-err").textContent = String(e);
    $("setup-err").hidden = false;
  }
});

$("btn-setup").addEventListener("click", () => show("setup"));

$("btn-upload").addEventListener("click", async () => {
  $("upload-err").hidden = true;
  $("upload-ok").hidden = true;
  $("btn-upload").disabled = true;
  try {
    const r = await invoke("upload_now");
    $("last-upload").textContent = r.at;
    $("last-status").textContent = r.status;
    $("upload-ok").textContent = `Found ${r.found}, sent ${r.sent}.`;
    $("upload-ok").hidden = false;
  } catch (e) {
    $("upload-err").textContent = String(e);
    $("upload-err").hidden = false;
  } finally {
    $("btn-upload").disabled = false;
  }
});

$("btn-check").addEventListener("click", async () => {
  $("addon-err").hidden = true;
  try { await refreshAddon(); } catch (e) {
    $("addon-err").textContent = String(e);
    $("addon-err").hidden = false;
  }
});

$("btn-update").addEventListener("click", async () => {
  $("addon-err").hidden = true;
  $("addon-ok").hidden = true;
  $("btn-update").disabled = true;
  try {
    const ver = await invoke("update_addon");
    $("addon-ok").textContent = `Installed ${ver}.`;
    $("addon-ok").hidden = false;
    await refreshAddon();
  } catch (e) {
    $("addon-err").textContent = String(e);
    $("addon-err").hidden = false;
    $("btn-update").disabled = false;
  }
});

$("btn-browse").addEventListener("click", async () => {
  try {
    const selected = await invoke("pick_folder");
    if (selected) $("game-folder").value = selected;
  } catch (_) {}
});

listen("sv-changed", () => $("btn-upload").click());
listen("tray-upload", () => $("btn-upload").click());
listen("prefs-changed", () => refresh());

// Browser preview harness for Playwright / Vizz (no Tauri)
if (!window.__TAURI__ && !window.__TAURI_INTERNALS__) {
  window.__PREVIEW__ = true;
  const mock = {
    prefs: { siteUrl: "", gameFolder: "", startAtLogin: false, setupDone: false, lastUploadAt: null, lastStatus: null, lastAddonVersion: null, addonName: "" },
    hasToken: false,
    candidates: [
      "C:\\Program Files (x86)\\World of Warcraft\\_classic_era_",
      "/Applications/World of Warcraft/_classic_era_",
    ],
    addonInstalled: null,
    gameRunning: false,
    defaults: { siteUrl: "https://example.guild", addonName: "ExampleAddon", githubRepo: "org/repo" },
  };
  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        if (cmd === "get_snapshot") return structuredClone(mock);
        if (cmd === "save_setup") {
          if (!args.input.token && !mock.hasToken) throw "paste an uploader token from the site (Settings → Uploader tokens)";
          if (args.input.token && !args.input.token.startsWith("upl_")) throw "token should start with upl_";
          if (!args.input.gameFolder) throw "that folder doesn't look like a game client (need WTF or Interface/AddOns)";
          Object.assign(mock.prefs, {
            siteUrl: args.input.siteUrl,
            gameFolder: args.input.gameFolder,
            startAtLogin: args.input.startAtLogin,
            setupDone: true,
            lastStatus: "ready",
          });
          mock.hasToken = true;
          mock.addonInstalled = "0.1.1";
          return structuredClone(mock);
        }
        if (cmd === "upload_now") {
          mock.prefs.lastUploadAt = new Date().toISOString();
          mock.prefs.lastStatus = "sent 1";
          return { found: 1, sent: 1, lines: ["gbank accepted"], at: mock.prefs.lastUploadAt, status: "sent 1" };
        }
        if (cmd === "check_addon") return { installed: "0.1.1", latest: "0.1.2", tag: "addon-v0.1.2", updateAvailable: true, gameRunning: false };
        if (cmd === "update_addon") { mock.addonInstalled = "0.1.2"; return "0.1.2"; }
        if (cmd === "start_watch") return null;
        throw new Error("unknown " + cmd);
      },
    },
    event: { listen: async () => () => {} },
    dialog: { open: async () => null },
  };
  // Demo toggle via ?screen=main|setup and ?theme=dark via class
  const q = new URLSearchParams(location.search);
  if (q.get("theme") === "dark") document.documentElement.style.colorScheme = "dark";
  if (q.get("theme") === "light") document.documentElement.style.colorScheme = "light";
  if (q.get("screen") === "main") {
    mock.prefs.setupDone = true;
    mock.hasToken = true;
    mock.prefs.lastUploadAt = "2026-11-04T20:00:00Z";
    mock.prefs.lastStatus = "sent 2";
    mock.addonInstalled = "0.1.1";
  }
}

refresh().catch((e) => {
  $("setup-err").textContent = String(e);
  $("setup-err").hidden = false;
  show("setup");
});
