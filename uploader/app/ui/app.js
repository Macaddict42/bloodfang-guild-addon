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

const TYPE_LABELS = {
  gbank: "guild bank snapshot",
  recipes: "profession recipes",
  signup: "raid signup",
  signups: "raid signup",
  loot: "loot",
  attendance: "attendance",
};

/** Format Unix seconds, ISO, or Date-parseable strings as local / relative time. */
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
  return d.toLocaleString(undefined, {
    day: "numeric",
    month: "short",
    hour: "numeric",
    minute: "2-digit",
  });
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
  if (!lines?.length) return "";
  const labels = [];
  const seen = new Set();
  for (const line of lines) {
    const ty = String(line).split(/\s+/)[0].toLowerCase();
    const label = TYPE_LABELS[ty] || ty.replace(/_/g, " ");
    if (label === "?" || seen.has(label)) continue;
    seen.add(label);
    labels.push(label);
  }
  return labels.join(" · ");
}

function showErr(panelId, msgId, message, fieldId) {
  const panel = $(panelId);
  $(msgId).textContent = sentenceCase(String(message));
  panel.hidden = false;
  if (fieldId) {
    document.querySelectorAll("input.is-error").forEach((el) => el.classList.remove("is-error"));
    const f = $(fieldId);
    if (f) f.classList.add("is-error");
  }
}

function clearErr(panelId, fieldId) {
  $(panelId).hidden = true;
  if (fieldId) $(fieldId)?.classList.remove("is-error");
}

function sentenceCase(s) {
  const t = String(s).trim();
  if (!t) return t;
  return t.charAt(0).toUpperCase() + t.slice(1);
}

function show(id) {
  $("screen-setup").classList.toggle("hidden", id !== "setup");
  $("screen-main").classList.toggle("hidden", id !== "main");
}

let lastAddonCheck = null;
let addonUiState = "idle"; // idle | available | uptodate | updating | failed | deferred

function fillSetup(snap) {
  $("site-url").value = snap.prefs.siteUrl || snap.defaults.siteUrl || "";
  $("game-folder").value = snap.prefs.gameFolder || "";
  $("start-login").checked = !!snap.prefs.startAtLogin;
  $("token-saved").hidden = !snap.hasToken;
  const hint = $("token-hint");
  if (hint) {
    hint.textContent = snap.hasToken
      ? "Stored in the OS credential store, never in a file."
      : "Paste a token from the site (Settings → Uploader tokens). Stored in the OS credential store, never in a file.";
  }
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
  $("last-upload").textContent = formatUploadTime(snap.prefs.lastUploadAt);
  $("last-status").textContent = formatStatus(snap.prefs.lastStatus);
  $("addon-ver").textContent = snap.addonInstalled || snap.prefs.lastAddonVersion || "—";
  const what = humanizeLines(snap.prefs.lastSentLines);
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
  $("addon-banner").hidden = true;
  $("addon-state").hidden = true;
  $("addon-err").hidden = true;
  $("addon-actions").hidden = false;
  const busy = state === "updating";
  $("btn-check").disabled = busy;
  $("btn-upload").disabled = busy;
  $("btn-addon-update").disabled = busy;
  $("btn-addon-later").disabled = busy;
  $("btn-addon-skip").disabled = busy;

  if (c) {
    $("addon-installed").textContent = c.installed || "not installed";
    $("addon-latest").textContent = c.latest || "—";
    $("addon-ver").textContent = c.installed || "—";
    $("addon-warn").hidden = !c.gameRunning;
  }

  if (state === "updating") {
    $("addon-state").textContent = "Updating…";
    $("addon-state").className = "note";
    $("addon-state").hidden = false;
    return;
  }
  if (state === "failed") {
    // err panel already filled by caller
    $("addon-actions").hidden = false;
    return;
  }
  if (state === "available" && c?.updateAvailable) {
    $("addon-banner-body").textContent =
      `Install ${c.latest} now? Takes effect after the next game start or /reload.`;
    $("addon-banner").hidden = false;
    $("addon-actions").hidden = true;
    return;
  }
  if (state === "deferred") {
    $("addon-state").textContent = "Update deferred. Check again when ready.";
    $("addon-state").className = "note";
    $("addon-state").hidden = false;
    return;
  }
  // uptodate / idle with no update
  $("addon-state").textContent = "Up to date.";
  $("addon-state").className = "note";
  $("addon-state").hidden = false;
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
  if (c.updateAvailable) setAddonUi("available", c);
  else setAddonUi("uptodate", c);
  return c;
}

$("btn-save").addEventListener("click", async () => {
  clearErr("setup-err", "token");
  $("site-url").classList.remove("is-error");
  $("game-folder").classList.remove("is-error");
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
    fillSetup(snap);
    show("main");
    await invoke("start_watch");
    await refreshAddon();
  } catch (e) {
    const msg = String(e);
    let field = "token";
    if (/folder|game client/i.test(msg)) field = "game-folder";
    else if (/site|url/i.test(msg)) field = "site-url";
    showErr("setup-err", "setup-err-msg", msg, field);
  }
});

$("btn-setup").addEventListener("click", async () => {
  try {
    const snap = await invoke("get_snapshot");
    fillSetup(snap);
  } catch (_) {}
  clearErr("setup-err", "token");
  show("setup");
});

$("btn-upload").addEventListener("click", async () => {
  clearErr("upload-err");
  $("upload-ok").hidden = true;
  $("btn-upload").disabled = true;
  try {
    const r = await invoke("upload_now");
    $("last-upload").textContent = formatUploadTime(r.at);
    $("last-status").textContent = formatStatus(r.status, r.sent);
    const what = humanizeLines(r.lines);
    if (what) {
      $("what-sent-body").textContent = what;
      $("what-sent").hidden = false;
    }
    $("upload-ok").textContent = r.sent ? `Sent ${r.sent}.` : "Nothing new.";
    $("upload-ok").hidden = false;
  } catch (e) {
    showErr("upload-err", "upload-err-msg", e);
  } finally {
    if (addonUiState !== "updating") $("btn-upload").disabled = false;
  }
});

$("btn-check").addEventListener("click", async () => {
  clearErr("addon-err");
  try { await refreshAddon(); } catch (e) {
    setAddonUi("failed", lastAddonCheck);
    showErr("addon-err", "addon-err-msg", e);
  }
});

async function runAddonUpdate() {
  clearErr("addon-err");
  setAddonUi("updating", lastAddonCheck);
  try {
    const ver = await invoke("update_addon");
    await refreshAddon();
    $("addon-state").textContent = `Installed ${ver}.`;
    $("addon-state").className = "note ok";
    $("addon-state").hidden = false;
  } catch (e) {
    setAddonUi("failed", lastAddonCheck);
    showErr("addon-err", "addon-err-msg", e);
  }
}

$("btn-addon-update").addEventListener("click", () => runAddonUpdate());
$("btn-addon-later").addEventListener("click", () => {
  setAddonUi("deferred", lastAddonCheck);
});
$("btn-addon-skip").addEventListener("click", async () => {
  clearErr("addon-err");
  try {
    const ver = lastAddonCheck?.latest;
    if (ver) await invoke("skip_addon_version", { version: ver });
    await refreshAddon();
  } catch (e) {
    setAddonUi("failed", lastAddonCheck);
    showErr("addon-err", "addon-err-msg", e);
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
  const q = new URLSearchParams(location.search);
  const os = q.get("os") || "win";
  const winCand = ["C:\\Program Files (x86)\\World of Warcraft\\_classic_era_"];
  const macCand = ["/Applications/World of Warcraft/_classic_era_"];
  const mock = {
    prefs: {
      siteUrl: "",
      gameFolder: "",
      startAtLogin: false,
      setupDone: false,
      lastUploadAt: null,
      lastStatus: null,
      lastAddonVersion: null,
      lastSentLines: null,
      skippedAddonVersion: null,
      addonName: "",
    },
    hasToken: false,
    candidates: os === "mac" ? macCand : winCand,
    addonInstalled: null,
    gameRunning: false,
    defaults: { siteUrl: "https://example.guild", addonName: "ExampleAddon", githubRepo: "org/repo" },
  };
  let addonFailNext = false;
  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        if (cmd === "get_snapshot") return structuredClone(mock);
        if (cmd === "save_setup") {
          if (!args.input.token && !mock.hasToken) {
            throw "Paste an uploader token from the site (Settings → Uploader tokens).";
          }
          if (args.input.token && !args.input.token.startsWith("upl_")) {
            throw "Token should start with upl_.";
          }
          if (!args.input.gameFolder) {
            throw "That folder doesn't look like a game client (need WTF or Interface/AddOns).";
          }
          Object.assign(mock.prefs, {
            siteUrl: args.input.siteUrl,
            gameFolder: args.input.gameFolder,
            startAtLogin: args.input.startAtLogin,
            setupDone: true,
            lastStatus: "ready",
          });
          mock.hasToken = true;
          mock.addonInstalled = mock.addonInstalled || "0.1.1";
          return structuredClone(mock);
        }
        if (cmd === "upload_now") {
          const secs = Math.floor(Date.now() / 1000);
          mock.prefs.lastUploadAt = String(secs);
          mock.prefs.lastStatus = "sent 2";
          mock.prefs.lastSentLines = ["gbank accepted", "signup accepted"];
          return {
            found: 2,
            sent: 2,
            lines: mock.prefs.lastSentLines,
            at: mock.prefs.lastUploadAt,
            status: "sent 2",
          };
        }
        if (cmd === "check_addon") {
          const installed = mock.addonInstalled || "0.1.1";
          const latest = mock._latest || "0.1.2";
          const skipped = mock.prefs.skippedAddonVersion;
          const updateAvailable =
            mock._forceUpdate === false
              ? false
              : skipped === latest
                ? false
                : (mock._forceUpdate === true || installed !== latest);
          return {
            installed,
            latest,
            tag: `addon-v${latest}`,
            updateAvailable,
            gameRunning: !!mock.gameRunning,
          };
        }
        if (cmd === "update_addon") {
          if (addonFailNext || mock._failUpdate) {
            addonFailNext = false;
            throw "Couldn't download the release. Check your network and try again.";
          }
          await new Promise((r) => setTimeout(r, mock._updateDelay || 0));
          mock.addonInstalled = mock._latest || "0.1.2";
          mock.prefs.lastAddonVersion = mock.addonInstalled;
          return mock.addonInstalled;
        }
        if (cmd === "skip_addon_version") {
          mock.prefs.skippedAddonVersion = args.version;
          return null;
        }
        if (cmd === "start_watch") return null;
        throw new Error("unknown " + cmd);
      },
    },
    event: { listen: async () => () => {} },
    dialog: { open: async () => null },
  };

  const screen = q.get("screen") || "setup";
  const addon = q.get("addon") || "available";

  if (screen === "main" || screen === "setup-token" || screen.startsWith("main")) {
    mock.prefs.setupDone = true;
    mock.hasToken = true;
    mock.prefs.siteUrl = "https://example.guild";
    mock.prefs.gameFolder = mock.candidates[0];
  }
  if (screen === "main" || screen.startsWith("main")) {
    // Unix seconds ~ 4 Nov 2026 12:00 local-ish demo
    mock.prefs.lastUploadAt = "1762254000";
    mock.prefs.lastStatus = "sent 2";
    mock.prefs.lastSentLines = ["gbank accepted", "signup accepted"];
    mock.addonInstalled = "0.1.1";
  }
  if (addon === "uptodate") {
    mock.addonInstalled = "0.1.2";
    mock._latest = "0.1.2";
    mock._forceUpdate = false;
    mock.prefs.lastUploadAt = null;
    mock.prefs.lastStatus = null;
    mock.prefs.lastSentLines = null;
  } else if (addon === "available") {
    mock.addonInstalled = "0.1.1";
    mock._latest = "0.1.2";
    mock._forceUpdate = true;
  } else if (addon === "updating") {
    mock.addonInstalled = "0.1.1";
    mock._latest = "0.1.2";
    mock._forceUpdate = true;
    mock._updateDelay = 60000;
  } else if (addon === "failed") {
    mock.addonInstalled = "0.1.1";
    mock._latest = "0.1.2";
    mock._forceUpdate = true;
    mock._failUpdate = true;
  }

  // After first paint, drive special screens
  window.__PREVIEW_BOOT__ = async () => {
    await refresh();
    if (screen === "setup-error") {
      show("setup");
      fillSetup(await invoke("get_snapshot"));
      $("site-url").value = "https://example.guild";
      showErr(
        "setup-err",
        "setup-err-msg",
        "Paste an uploader token from the site (Settings → Uploader tokens).",
        "token",
      );
    } else if (screen === "setup-token") {
      show("setup");
      fillSetup(await invoke("get_snapshot"));
    } else if (addon === "updating") {
      setAddonUi("available", await invoke("check_addon"));
      setAddonUi("updating", lastAddonCheck);
    } else if (addon === "failed") {
      setAddonUi("available", await invoke("check_addon"));
      setAddonUi("failed", lastAddonCheck);
      showErr(
        "addon-err",
        "addon-err-msg",
        "Couldn't download the release. Check your network and try again.",
      );
    } else if (addon === "deferred") {
      setAddonUi("deferred", await invoke("check_addon"));
    }
  };
}

const boot = window.__PREVIEW_BOOT__
  ? window.__PREVIEW_BOOT__()
  : refresh();

boot.catch((e) => {
  showErr("setup-err", "setup-err-msg", e, "token");
  show("setup");
});
