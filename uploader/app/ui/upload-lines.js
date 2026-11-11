/* Per-string ingest lines. The site's error text is shown as sent.
   A refusal does not mean the saved sign-in is dead. */
(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  root.uploaderLines = api;
})(typeof globalThis !== "undefined" ? globalThis : this, function () {
  const TYPE_LABELS = {
    gbank: "Guild bank",
    recipes: "Recipes",
    signup: "Raid signup",
    signups: "Raid signup",
    loot: "Loot",
    attendance: "Attendance",
    ranks: "Rank names",
  };

  const SENT_UNITS = {
    ranks: { one: "rank", many: "ranks" },
    gbank: { one: "item", many: "items" },
    recipes: { one: "recipe", many: "recipes" },
  };

  const SENT_ORDER = ["ranks", "gbank", "recipes"];

  function partitionUploadLines(lines) {
    const accepted = [];
    const refused = [];
    for (const line of lines || []) {
      const text = String(line);
      const sp = text.indexOf(" ");
      const ty = (sp === -1 ? text : text.slice(0, sp)).toLowerCase();
      const rest = sp === -1 ? "" : text.slice(sp + 1);
      if (rest === "accepted" || rest === "duplicate" || /^(accepted|duplicate) \d+$/.test(rest)) accepted.push(text);
      else if (rest) refused.push({ ty, error: rest });
    }
    return { accepted, refused };
  }

  function typeOfLine(line) {
    return String(line).split(/\s+/)[0].toLowerCase();
  }

  function humanizeUploadLines(lines) {
    const labels = [];
    const seen = new Set();
    for (const line of partitionUploadLines(lines).accepted) {
      const ty = typeOfLine(line);
      const label = TYPE_LABELS[ty] || ty.replace(/_/g, " ");
      if (label === "?" || seen.has(label)) continue;
      seen.add(label);
      labels.push(label);
    }
    return labels.join(" · ");
  }

  function sentenceCase(s) {
    const t = String(s).trim();
    if (!t) return t;
    return t.charAt(0).toUpperCase() + t.slice(1);
  }

  function refusalSentence(ty, error) {
    const e = String(error).trim();
    if (/\s/.test(e)) {
      const body = /[.!?]$/.test(e) ? e : `${e}.`;
      return sentenceCase(body);
    }
    if (e === "site_refused" || e === "forbidden") {
      return ty === "gbank" ? "The site didn't accept the guild bank scan." : "The site didn't accept this.";
    }
    if (e === "bad_envelope") return "The site couldn't use this scan.";
    if (e === "unknown_type") return "The site doesn't take this kind of scan yet.";
    return "The site didn't accept this.";
  }

  function withSignInSaved(sentence) {
    return /this sign-in is still saved/i.test(sentence)
      ? sentence
      : `${sentence} This sign-in is still saved.`;
  }

  function refusalMessage(refused) {
    const message = refused.map((row) => withSignInSaved(refusalSentence(row.ty, row.error))).join(" ");
    const title = refused.some((row) => row.ty === "gbank")
      ? "Couldn't send guild bank"
      : refused.every((row) => row.ty === "ranks")
        ? "Rank names not sent"
        : "Some strings were not accepted";
    return { title, message, dropsToken: false };
  }

  function uploadFailureDropsToken(message) {
    return /rejected the uploader token/i.test(String(message));
  }

  function uploadFailureMessage(message) {
    const m = String(message || "").trim();
    if (/couldn'?t reach the site/i.test(m)) {
      return "Couldn't reach the site. This sign-in is still saved. The uploader will try again on its own.";
    }
    if (!m) return "Couldn't send. This sign-in is still saved.";
    return withSignInSaved(sentenceCase(/[.!?]$/.test(m) ? m : `${m}.`));
  }

  /* "Last checked" shows only when a later pass sent nothing (unix-second strings). */
  function showLastChecked(uploadAt, checkedAt) {
    const c = Number(checkedAt);
    if (!checkedAt || !Number.isFinite(c)) return false;
    const u = Number(uploadAt);
    return !uploadAt || !Number.isFinite(u) || c > u;
  }

  function formatUploadTime(raw, now, locale) {
    if (raw == null || raw === "" || raw === "—") return "Never";
    let d;
    if (typeof raw === "number" || /^\d{9,12}$/.test(String(raw).trim())) {
      d = new Date(Number(raw) * 1000);
    } else {
      d = new Date(raw);
    }
    if (Number.isNaN(d.getTime())) return String(raw);
    const opts = { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" };
    const current = now ? new Date(now) : new Date();
    if (d.getFullYear() !== current.getFullYear()) opts.year = "numeric";
    return d.toLocaleString(locale, opts);
  }

  function formatStatus(opts) {
    const o = opts || {};
    if (o.sending) return "Sending…";
    if (o.failed) return "Couldn't send";
    if (o.checkedOnly) return "Nothing new to send";
    const accepted = o.acceptedCount;
    const refused = o.refusedCount || 0;
    if (typeof accepted === "number") {
      if (accepted === 0 && refused === 0) return "Nothing new to send";
      if (accepted === 0 && refused > 0) return "Couldn't send";
      return refused > 0 ? "Sent, with an error" : "Sent";
    }
    const raw = o.raw;
    if (raw == null || raw === "" || raw === "—" || raw === "Waiting for SavedVariables") return "Not uploaded yet";
    const m = String(raw).match(/^sent\s+(\d+)$/i);
    if (m) {
      const n = Number(m[1]);
      return n === 0 ? "Nothing new to send" : "Sent";
    }
    if (/^nothing new( to send)?$/i.test(String(raw))) return "Nothing new to send";
    if (String(raw).toLowerCase() === "ready") return "Not uploaded yet";
    return String(raw);
  }

  function sentLine(ty, n) {
    const label = TYPE_LABELS[ty] || String(ty).replace(/_/g, " ");
    const units = SENT_UNITS[ty];
    if (!units || n == null || !Number.isFinite(Number(n))) return label;
    const count = Number(n);
    const unit = count === 1 ? units.one : units.many;
    return `${label}: ${count} ${unit}`;
  }

  function countMap(counts) {
    const map = new Map();
    for (const row of counts || []) {
      if (!row) continue;
      const ty = String(row.ty || row.type || "").toLowerCase();
      const n = Number(row.n);
      if (!ty || !Number.isFinite(n)) continue;
      map.set(ty, (map.get(ty) || 0) + n);
    }
    return map;
  }

  function sentList(lines, counts) {
    const parts = partitionUploadLines(lines);
    const map = countMap(counts);
    const seen = new Set();
    const types = [];
    for (const line of parts.accepted) {
      const ty = typeOfLine(line);
      if (!ty || ty === "?" || seen.has(ty)) continue;
      seen.add(ty);
      types.push(ty);
    }
    types.sort((a, b) => {
      const ia = SENT_ORDER.indexOf(a);
      const ib = SENT_ORDER.indexOf(b);
      return (ia === -1 ? 99 : ia) - (ib === -1 ? 99 : ib);
    });
    return types.map((ty) => sentLine(ty, map.has(ty) ? map.get(ty) : null));
  }

  function sentHeading(opts) {
    const o = opts || {};
    if (o.failed) return null;
    if (o.sending || o.checkedOnly) return "Last sent";
    return "What was sent";
  }

  return {
    TYPE_LABELS,
    showLastChecked,
    partitionUploadLines,
    humanizeUploadLines,
    refusalMessage,
    uploadFailureDropsToken,
    uploadFailureMessage,
    formatUploadTime,
    formatStatus,
    sentLine,
    sentList,
    sentHeading,
  };
});
