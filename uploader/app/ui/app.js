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

const linesApi = () => window.uploaderLines;

function formatUploadTime(raw) {
  if (raw == null || raw === "" || raw === "—") return "Never";
  let d;
  if (typeof raw === "number" || /^\d{9,12}$/.test(String(raw).trim())) {
    d = new Date(Number(raw) * 1000);
  } else {
    d = new Date(raw);
  }
  if (Number.isNaN(d.getTime())) return String(raw);
  const now = Date.now();
  const diffSec = Math.round((now - d.getTime()) / 1000);
  if (diffSec >= 0 && diffSec < 60) return "Just now";
  if (diffSec >= 60 && diffSec < 3600) {
    const m = Math.floor(diffSec / 60);
    return m === 1 ? "1 minute ago" : `${m} minutes ago`;
  }
  if (diffSec >= 3600 && diffSec < 86400) {
    const h = Math.floor(diffSec / 3600);
    return h === 1 ? "1 hour ago" : `${h} hours ago`;
  }
  if (diffSec >= 86400 && diffSec < 172800) return "Yesterday";
  return d.toLocaleString(undefined, { day: "numeric", month: "short", hour: "numeric", minute: "2-digit" });
}

function formatStatus(raw, sent) {
  if (raw == null || raw === "" || raw === "—") return "Waiting for SavedVariables";
  const m = String(raw).match(/^sent\s+(\d+)$/i);
  if (m) {
    const n = Number(m[1]);
    return n === 0 ? "Nothing new" : `Sent ${n}`;
  }
  if (String(raw).toLowerCase() === "nothing new") return "Nothing new";
  if (String(raw).toLowerCase() === "ready") return "Ready";
  if (typeof sent === "number") return sent === 0 ? "Nothing new" : `Sent ${sent}`;
  return String(raw);
}

function humanizeLines(lines) {
  return linesApi().humanizeUploadLines(lines);
}

function showRecovery(phase) {
  const el = $("connect-recovery");
  if (!phase) {
    el.hidden = true;
    return null;
  }
  const panel = window.connectRecovery.connectPhase(phase);
  el.textContent = panel.recovery;
  el.hidden = false;
  return panel;
}

function showErr(panelId, msgId, message, title) {
  const panel = $(panelId);
  $(msgId).textContent = sentenceCase(String(message));
  const strong = panel.querySelector("strong");
  if (strong && title) strong.textContent = title;
  panel.hidden = false;
}

function clearErr(panelId) {
  $(panelId).hidden = true;
}

function sentenceCase(s) {
  const t = String(s).trim();
  if (!t) return t;
  return t.charAt(0).toUpperCase() + t.slice(1);
}

function setState(name) {
  document.getElementById("app").dataset.state = name;
}

function show(id) {
  $("screen-boot").classList.toggle("hidden", id !== "boot");
  $("screen-setup").classList.toggle("hidden", id !== "setup");
  $("screen-main").classList.toggle("hidden", id !== "main");
  setState(id);
}

let lastAddonCheck = null;
let addonUiState = "idle";
let hasToken = false;
let folderResolved = false;
let resolvedFolder = "";
let userPickedFolder = false;
let connectInFlight = false;
let startAtLogin = false;

function setLoginBox(on) {
  startAtLogin = !!on;
  $("start-login").checked = startAtLogin;
  $("start-login-main").checked = startAtLogin;
}

function showFolder(path) {
  resolvedFolder = path || "";
  const label = $("game-folder-label");
  if (path) {
    label.hidden = false;
    label.textContent = path;
    $("folder-main").hidden = false;
    $("folder-main-body").textContent = path;
  } else {
    label.hidden = true;
    $("folder-main").hidden = true;
  }
}

function setFolderUi(showUi, choices) {
  $("folder-block").hidden = !showUi;
  const box = $("folder-choices");
  box.innerHTML = "";
  if (choices?.length) {
    box.hidden = false;
    choices.forEach((p) => {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = p;
      b.addEventListener("click", () => chooseFolder(p));
      box.appendChild(b);
    });
  } else {
    box.hidden = true;
  }
}

function fillMainFromPrefs(prefs) {
  $("last-upload").textContent = formatUploadTime(prefs.lastUploadAt);
  $("last-status").textContent = formatStatus(prefs.lastStatus);
  $("addon-ver").textContent = prefs.lastAddonVersion || "—";
  const what = humanizeLines(prefs.lastSentLines);
  if (what) {
    $("what-sent-body").textContent = what;
    $("what-sent").hidden = false;
  } else {
    $("what-sent").hidden = true;
  }
}

function setAddonUi(state, check) {
  addonUiState = state;
  lastAddonCheck = check || lastAddonCheck;
  const c = lastAddonCheck;
  $("addon-err").hidden = state !== "failed";
  if (c) {
    $("addon-installed").textContent = c.installed || "not installed";
    $("addon-latest").textContent = c.latest || "—";
    $("addon-ver").textContent = c.installed || "—";
    $("addon-warn").hidden = !c.gameRunning;
  }
  let text = "Checking the addon…";
  if (state === "updating") text = "Updating the addon…";
  else if (state === "failed") text = "The addon update did not finish.";
  else if (c?.gameRunning && c?.updateAvailable) text = "Game is running. The addon update will wait until it closes.";
  else if (c?.latest) text = "Up to date.";
  else text = "Couldn't check for an addon update.";
  $("addon-state").textContent = text;
  $("addon-state").hidden = false;
}

let lastAppOffer = null;

function applyAppOffer(check) {
  lastAppOffer = check || null;
  const showOffer = check?.offer === true;
  $("app-update").hidden = !showOffer;
  if (showOffer) $("app-update-note").hidden = true;
}

function maybeMain() {
  if (hasToken && folderResolved && !connectInFlight) {
    show("main");
    setState("main");
    invoke("start_watch").catch(() => {});
    invoke("check_addon").then((c) => {
      if (c.gameRunning && c.updateAvailable) setAddonUi("deferred", c);
      else if (c.updateAvailable) setAddonUi("updating", c);
      else setAddonUi("uptodate", c);
    }).catch((e) => {
      setAddonUi("failed", lastAddonCheck);
      showErr("addon-err", "addon-err-msg", e);
    });
    invoke("check_self_update").then(applyAppOffer).catch(() => applyAppOffer(null));
    invoke("game_running").then((g) => {
      if (lastAddonCheck) lastAddonCheck.gameRunning = !!g.running;
      $("addon-warn").hidden = !g.running;
    }).catch(() => {});
    return;
  }
  show("setup");
  if (!folderResolved) setState("needs-folder");
  else if (!hasToken) setState("needs-sign-in");
  else setState("setup");
}

function applyToken(status) {
  hasToken = status?.hasToken === true && status?.state === "present";
  $("btn-connect").disabled = hasToken || connectInFlight;
  $("btn-connect").hidden = hasToken;
  if (status?.state === "unavailable") {
    showRecovery(false);
    $("connect-state").textContent = "Couldn't read the saved sign-in. You can try again.";
  } else if (hasToken) {
    showRecovery(false);
    $("connect-state").textContent = "Signed in. The token stays in the system credential store.";
  } else if (!connectInFlight && $("setup-err").hidden) {
    showRecovery(false);
    $("connect-state").textContent = "Not signed in.";
  }
  maybeMain();
}

async function chooseFolder(path) {
  userPickedFolder = true;
  clearErr("setup-err");
  try {
    await invoke("save_setup", { input: { gameFolder: path, startAtLogin } });
    folderResolved = true;
    showFolder(path);
    $("folder-block").hidden = false;
    $("folder-hint").textContent = "Using this folder.";
    $("game-folder-label").hidden = false;
    $("folder-choices").hidden = true;
    maybeMain();
  } catch (e) {
    folderResolved = false;
    showErr("setup-err", "setup-err-msg", e);
    show("setup");
  }
}

async function boot() {
  show("boot");
  let bootState;
  try {
    bootState = await invoke("get_boot");
  } catch (e) {
    show("setup");
    showErr("setup-err", "setup-err-msg", e);
    $("btn-connect").disabled = false;
    return;
  }
  setLoginBox(bootState.prefs.startAtLogin);
  fillMainFromPrefs(bootState.prefs);
  const host = bootState.siteHost || "";
  $("site-line").textContent = bootState.siteConfigured
    ? `Opens your browser for ${host}. You don't type an address or a token.`
    : "This build has no site address, so sign-in and upload are off.";
  if (!bootState.prefsReadable) {
    showErr("setup-err", "setup-err-msg", "Saved settings couldn't be read, so they were left unchanged.");
  }
  if (!bootState.siteConfigured) $("btn-connect").disabled = true;
  show("setup");
  $("connect-state").textContent = "Checking saved sign-in…";

  invoke("token_status").then(applyToken).catch(() => applyToken({ hasToken: false, state: "unavailable" }));
  invoke("probe_game_folder").then((probe) => {
    if (userPickedFolder) return;
    if (probe.resolved && probe.showFolderUi === false) {
      folderResolved = true;
      showFolder(probe.resolved);
      setFolderUi(false, []);
      invoke("accept_detected_folder", { folder: probe.resolved }).catch((e) => {
        showErr("setup-err", "setup-err-msg", e);
      });
    } else {
      folderResolved = false;
      showFolder("");
      const choices = probe.choices || [];
      $("folder-hint").textContent = choices.length
        ? "More than one game folder was found. Pick one — only needed when auto-detect isn't sure. This does not search the whole disk."
        : "Couldn't find the usual Forever install. Choose the game folder once — only needed when auto-detect misses. This does not search the whole disk.";
      setFolderUi(true, choices);
      setState("needs-folder");
    }
    maybeMain();
  }).catch(() => {
    if (userPickedFolder) return;
    folderResolved = false;
    setFolderUi(true, []);
    show("setup");
    setState("needs-folder");
  });
}

function beginWait() {
  clearErr("setup-err");
  connectInFlight = true;
  const panel = showRecovery("waiting");
  $("btn-connect").disabled = true;
  $("btn-switch").hidden = panel.switchHidden;
  $("btn-cancel-connect").hidden = panel.cancelHidden;
  $("connect-state").textContent = panel.status;
  show("setup");
  setState("waiting");
}

function failConnect(message) {
  connectInFlight = false;
  hasToken = false;
  $("btn-cancel-connect").hidden = true;
  $("btn-connect").hidden = false;
  $("btn-connect").disabled = false;
  $("btn-switch").hidden = false;
  const text = String(message);
  const cancelled = /cancelled/i.test(text);
  if (cancelled) {
    const panel = showRecovery("cancelled");
    $("btn-cancel-connect").hidden = panel.cancelHidden;
    $("btn-switch").hidden = panel.switchHidden;
    $("connect-state").textContent = panel.status;
  } else {
    showRecovery(false);
    $("connect-state").textContent = "Nothing was saved.";
  }
  const title = /right role/i.test(text) ? "Couldn't sign in" : "Couldn't continue";
  showErr("setup-err", "setup-err-msg", text, title);
  show("setup");
  setState(cancelled ? "cancelled" : "sign-in-failed");
}

async function startConnect(switchAccount) {
  beginWait();
  try {
    const outcome = await invoke("connect_browser", { switchAccount: !!switchAccount });
    connectInFlight = false;
    $("btn-cancel-connect").hidden = true;
    $("btn-switch").hidden = true;
    applyToken({ hasToken: outcome.hasToken === true, state: outcome.hasToken ? "present" : "absent" });
  } catch (e) {
    failConnect(e);
  }
}

$("btn-connect").addEventListener("click", () => startConnect(false));
$("btn-switch").addEventListener("click", () => startConnect(true));

$("btn-cancel-connect").addEventListener("click", () => {
  invoke("cancel_connect").catch(() => {});
});

async function persistLogin(enabled) {
  setLoginBox(enabled);
  try {
    await invoke("set_start_at_login", { enabled });
  } catch (e) {
    showErr("setup-err", "setup-err-msg", e);
  }
}

$("start-login").addEventListener("change", () => persistLogin($("start-login").checked));
$("start-login-main").addEventListener("change", () => persistLogin($("start-login-main").checked));

$("btn-browse").addEventListener("click", async () => {
  try {
    const selected = await invoke("pick_folder");
    if (selected) await chooseFolder(selected);
  } catch (e) {
    showErr("setup-err", "setup-err-msg", e);
  }
});

$("btn-change-folder").addEventListener("click", async () => {
  try {
    const selected = await invoke("pick_folder");
    if (selected) await chooseFolder(selected);
  } catch (e) {
    showErr("upload-err", "upload-err-msg", e);
  }
});

$("btn-disconnect").addEventListener("click", async () => {
  clearErr("upload-err");
  try {
    const status = await invoke("clear_token");
    applyToken(status);
    show("setup");
  } catch (e) {
    showErr("upload-err", "upload-err-msg", e);
  }
});

function showUploadSuccess(r) {
  clearErr("upload-err");
  $("last-upload").textContent = formatUploadTime(r.at);
  const parts = linesApi().partitionUploadLines(r.lines);
  $("last-status").textContent = formatStatus(r.status, parts.accepted.length);
  const what = humanizeLines(parts.accepted);
  if (what) {
    $("what-sent-body").textContent = what;
    $("what-sent").hidden = false;
  }
  if (parts.accepted.length) {
    $("upload-ok").textContent = `Sent ${parts.accepted.length}.`;
    $("upload-ok").hidden = false;
  }
  if (parts.refused.length) {
    const note = linesApi().refusalMessage(parts.refused);
    showErr("upload-err", "upload-err-msg", note.message, note.title);
  }
}

function showUploadFailure(message) {
  const msg = String(message || "");
  showErr("upload-err", "upload-err-msg", msg, "Upload failed");
  if (linesApi().uploadFailureDropsToken(msg)) {
    hasToken = false;
    applyToken({ hasToken: false, state: "absent" });
  }
}

$("btn-app-later").addEventListener("click", async () => {
  const version = lastAppOffer?.latest;
  if (!version) return;
  try {
    await invoke("defer_app_update", { version });
    $("app-update").hidden = true;
    $("app-update-note").hidden = true;
  } catch (e) {
    $("app-update-note").hidden = false;
    $("app-update-note").textContent = String(e);
  }
});

$("btn-app-update").addEventListener("click", async () => {
  $("btn-app-update").disabled = true;
  try {
    const result = await invoke("accept_app_update");
    $("app-update-note").hidden = false;
    $("app-update-note").textContent = result?.message || "Nothing was downloaded or restarted.";
  } catch (e) {
    $("app-update-note").hidden = false;
    $("app-update-note").textContent = String(e);
  } finally {
    $("btn-app-update").disabled = false;
  }
});

listen("addon-auto", (event) => {
  const r = event?.payload;
  if (!r) return;
  setAddonUi(r.ok ? "uptodate" : "failed", {
    installed: r.installed,
    latest: r.latest,
    updateAvailable: false,
    gameRunning: !!r.gameRunning,
  });
  if (r.message) {
    $("addon-state").textContent = r.message;
    $("addon-state").hidden = false;
  }
  if (!r.ok) showErr("addon-err", "addon-err-msg", r.message, "Update failed");
  else clearErr("addon-err");
});
listen("sv-changed", (event) => {
  const r = event?.payload;
  if (!r || typeof r !== "object" || !("ok" in r)) return;
  clearErr("upload-err");
  $("upload-ok").hidden = true;
  if (!r.ok) showUploadFailure(r.error);
  else showUploadSuccess(r);
});
listen("autostart-failed", (event) => {
  const msg = event?.payload || "Couldn't change start at login.";
  showErr("setup-err", "setup-err-msg", msg);
});

if (!window.__TAURI__ && !window.__TAURI_INTERNALS__) {
  window.__PREVIEW__ = true;
  const q = new URLSearchParams(location.search);
  const os = q.get("os") || "win";
  const screen = q.get("screen") || "signin";
  const winFolder = "C:\\Program Files (x86)\\World of Warcraft\\_forever_";
  const macChoices = [
    "/Applications/World of Warcraft/_forever_",
    "/Users/member/Games/World of Warcraft/_classic_",
  ];
  const mock = {
    prefs: {
      siteUrl: "",
      gameFolder: "",
      startAtLogin: screen === "existing-off" ? false : true,
      setupDone: false,
      lastUploadAt: null,
      lastStatus: null,
      lastAddonVersion: null,
      lastSentLines: null,
      skippedAddonVersion: null,
      addonName: "ExampleAddon",
    },
    hasToken: false,
    tokenState: "absent",
    folder: null,
    choices: [],
    siteConfigured: screen !== "no-site",
    siteHost: screen === "no-site" ? "" : "example.invalid",
    prefsReadable: screen !== "corrupt-prefs",
    origin: screen === "existing-off" ? "existingFile" : "firstInstall",
    gameRunning: q.get("running") === "1",
    addonInstalled: null,
    _latest: "0.1.2",
    _forceUpdate: true,
  };
  if (screen === "existing-off") {
    mock.hasToken = true;
    mock.tokenState = "present";
    mock.folder = os === "mac" ? macChoices[0] : winFolder;
    mock.prefs.gameFolder = mock.folder;
    mock.prefs.setupDone = true;
  }
  if (screen === "main" || screen === "app-update" || screen === "addon-busy" || screen === "denied" || screen === "expired" || screen === "cancelled" || screen === "retry" || screen === "waiting" || screen === "offline" || screen === "recovery") {
    mock.hasToken = screen === "main" || screen === "app-update" || screen === "addon-busy";
    mock.tokenState = mock.hasToken ? "present" : "absent";
    mock.folder = os === "mac" ? macChoices[0] : winFolder;
    mock.prefs.gameFolder = mock.folder;
    mock.prefs.setupDone = true;
    mock.prefs.lastUploadAt = "1762254000";
    mock.prefs.lastStatus = "sent 2";
    mock.prefs.lastSentLines = ["gbank accepted", "recipes accepted"];
    mock.addonInstalled = screen === "addon-busy" ? "0.1.1" : "0.1.2";
    mock._forceUpdate = false;
  }
  if (screen === "folder" || screen === "mac-choices") {
    mock.folder = null;
    mock.choices = screen === "mac-choices" || os === "mac" ? macChoices : [];
  }
  if (screen === "signin" || screen === "signin-found") {
    mock.folder = os === "mac" ? macChoices[0] : winFolder;
  }
  if (q.get("addon") === "uptodate") {
    mock.addonInstalled = "0.1.2";
    mock._latest = "0.1.2";
    mock._forceUpdate = false;
  }

  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        if (cmd === "get_boot") {
          if (screen === "slow-boot") await new Promise((r) => setTimeout(r, 400));
          return {
            prefs: structuredClone(mock.prefs),
            origin: mock.origin,
            siteConfigured: mock.siteConfigured,
            siteHost: mock.siteHost,
            prefsReadable: mock.prefsReadable,
          };
        }
        if (cmd === "token_status") {
          if (screen === "slow-token") await new Promise((r) => setTimeout(r, 800));
          return { hasToken: mock.hasToken, state: mock.tokenState };
        }
        if (cmd === "probe_game_folder") {
          if (screen === "slow-probe") await new Promise((r) => setTimeout(r, 800));
          if (mock.folder && !mock.choices.length) {
            return { resolved: mock.folder, source: os === "mac" ? "platformProbe" : "canonicalForever", showFolderUi: false, choices: [] };
          }
          return { resolved: null, source: "none", showFolderUi: true, choices: mock.choices };
        }
        if (cmd === "connect_browser") {
          if (args && args.switchAccount) mock._promptLogin = true;
          if (screen === "denied") throw "Sign-in was declined. Nothing was saved.";
          if (screen === "retry") throw "Sign-in expired or was already used. Try again.";
          if (screen === "expired") throw "Sign-in expired. Try again.";
          if (screen === "cancelled" || screen === "recovery") throw "Sign-in cancelled. Nothing was saved.";
          if (screen === "offline") throw "Couldn't reach the site to sign in. Nothing was saved.";
          mock.hasToken = true;
          mock.tokenState = "present";
          return { hasToken: true };
        }
        if (cmd === "cancel_connect") return null;
        if (cmd === "accept_detected_folder" || cmd === "save_setup") {
          const folder = args.folder || args.input?.gameFolder;
          if (!folder) throw "That folder doesn't look like a game client (need WTF, Interface/AddOns, or Wow.exe).";
          mock.folder = folder;
          mock.prefs.gameFolder = folder;
          mock.prefs.setupDone = true;
          if (args.input) mock.prefs.startAtLogin = !!args.input.startAtLogin;
          return null;
        }
        if (cmd === "set_start_at_login") {
          mock.prefs.startAtLogin = !!args.enabled;
          return null;
        }
        if (cmd === "clear_token") {
          mock.hasToken = false;
          mock.tokenState = "absent";
          return { hasToken: false, state: "absent" };
        }
        if (cmd === "pick_folder") {
          return os === "mac"
            ? "/Users/member/Games/World of Warcraft/_forever_"
            : "D:\\Program Files\\World of Warcraft\\_forever_";
        }
        if (cmd === "upload_now") {
          if (screen === "offline") throw "couldn't reach the site: offline";
          const secs = Math.floor(Date.now() / 1000);
          mock.prefs.lastUploadAt = String(secs);
          mock.prefs.lastStatus = "sent 2";
          mock.prefs.lastSentLines = ["gbank accepted"];
          return { found: 1, sent: 1, lines: mock.prefs.lastSentLines, at: mock.prefs.lastUploadAt, status: "sent 2" };
        }
        if (cmd === "check_addon") {
          if (screen === "offline") throw "couldn't reach GitHub: offline";
          const installed = mock.addonInstalled || "0.1.1";
          const latest = mock._latest || "0.1.2";
          return {
            installed,
            latest,
            tag: `addon-v${latest}`,
            updateAvailable: screen === "addon-busy" || (mock._forceUpdate === true && installed !== latest),
            gameRunning: screen === "addon-busy",
          };
        }
        if (cmd === "game_running") return { running: !!mock.gameRunning };
        if (cmd === "update_addon") {
          mock.addonInstalled = mock._latest;
          return mock.addonInstalled;
        }
        if (cmd === "skip_addon_version") {
          mock.prefs.skippedAddonVersion = args.version;
          mock._forceUpdate = false;
          return null;
        }
        if (cmd === "start_watch") return null;
        if (cmd === "check_self_update") {
          if (screen === "app-update") return { current: "0.1.0", latest: "0.2.0", updateAvailable: true, offer: true };
          return { current: "0.1.0", latest: "0.1.0", updateAvailable: false, offer: false };
        }
        if (cmd === "defer_app_update") return null;
        if (cmd === "accept_app_update") {
          return { installed: false, message: "This build can't install an app update. No verified installer is published, so nothing was downloaded or restarted." };
        }
        throw new Error("unknown " + cmd);
      },
    },
    event: { listen: async () => () => {} },
  };
}

const bootPromise = boot();
bootPromise.catch((e) => {
  showErr("setup-err", "setup-err-msg", e);
  show("setup");
});

if (window.__PREVIEW__) {
  const previewScreen = new URLSearchParams(location.search).get("screen");
  bootPromise.then(() => {
    if (previewScreen === "waiting") {
      setTimeout(() => beginWait(), 400);
    } else if (["denied", "expired", "cancelled", "offline", "retry", "recovery"].includes(previewScreen)) {
      $("btn-connect").disabled = false;
      $("btn-connect").click();
    }
  });
}
