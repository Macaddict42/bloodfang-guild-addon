/* Per-string ingest lines. The site's error text is shown as sent.
   A refusal does not mean the saved sign-in is dead. */
(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  root.uploaderLines = api;
})(typeof globalThis !== "undefined" ? globalThis : this, function () {
  const TYPE_LABELS = {
    gbank: "guild bank snapshot",
    recipes: "profession recipes",
    signup: "raid signup",
    signups: "raid signup",
    loot: "loot",
    attendance: "attendance",
  };

  function partitionUploadLines(lines) {
    const accepted = [];
    const refused = [];
    for (const line of lines || []) {
      const text = String(line);
      const sp = text.indexOf(" ");
      const ty = (sp === -1 ? text : text.slice(0, sp)).toLowerCase();
      const rest = sp === -1 ? "" : text.slice(sp + 1);
      if (rest === "accepted" || rest === "duplicate") accepted.push(text);
      else if (rest) refused.push({ ty, error: rest });
    }
    return { accepted, refused };
  }

  function humanizeUploadLines(lines) {
    const labels = [];
    const seen = new Set();
    for (const line of partitionUploadLines(lines).accepted) {
      const ty = String(line).split(/\s+/)[0].toLowerCase();
      const label = TYPE_LABELS[ty] || ty.replace(/_/g, " ");
      if (label === "?" || seen.has(label)) continue;
      seen.add(label);
      labels.push(label);
    }
    return labels.join(" · ");
  }

  function refusalMessage(refused) {
    const message = refused.map((row) => {
      const label = TYPE_LABELS[row.ty] || row.ty;
      return `${label}: ${row.error}. This sign-in is still saved.`;
    }).join(" ");
    const title = refused.some((row) => row.ty === "gbank")
      ? "Couldn't send guild bank"
      : "Some strings were not accepted";
    return { title, message, dropsToken: false };
  }

  function uploadFailureDropsToken(message) {
    return /rejected the uploader token/i.test(String(message));
  }

  return {
    TYPE_LABELS,
    partitionUploadLines,
    humanizeUploadLines,
    refusalMessage,
    uploadFailureDropsToken,
  };
});
