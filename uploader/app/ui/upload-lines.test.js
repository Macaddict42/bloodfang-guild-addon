const assert = require("assert");
const {
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
  showLastChecked,
} = require("./upload-lines.js");

const mixed = partitionUploadLines([
  "gbank site_refused",
  "recipes accepted",
]);
assert.deepStrictEqual(mixed.refused, [{ ty: "gbank", error: "site_refused" }]);
assert.deepStrictEqual(mixed.accepted, ["recipes accepted"]);
assert.strictEqual(humanizeUploadLines(["gbank site_refused", "recipes accepted"]), "Recipes");

const bank = refusalMessage(mixed.refused);
assert.strictEqual(bank.dropsToken, false);
assert.strictEqual(bank.title, "Couldn't send guild bank");
assert.match(bank.message, /The site didn't accept the guild bank scan\./);
assert.match(bank.message, /still saved/);
assert.doesNotMatch(bank.message, /site_refused/);
assert.doesNotMatch(bank.message, /Officer/);
assert.doesNotMatch(bank.message, /Recipes/);

const own = refusalMessage([{ ty: "recipes", error: "site_refused" }]);
assert.strictEqual(own.title, "Some strings were not accepted");
assert.match(own.message, /The site didn't accept this\./);
assert.doesNotMatch(own.message, /site_refused/);
assert.doesNotMatch(own.message, /Officer/);
assert.strictEqual(own.dropsToken, false);

assert.deepStrictEqual(partitionUploadLines(["gbank duplicate"]).refused, []);
assert.strictEqual(uploadFailureDropsToken("the site rejected the uploader token; make a new one"), true);
assert.strictEqual(uploadFailureDropsToken("The site didn't accept the guild bank scan. This sign-in is still saved."), false);
assert.strictEqual(uploadFailureDropsToken("Sign in again with a Discord account that has the right role."), false);
assert.strictEqual(
  uploadFailureMessage("couldn't reach the site: example.invalid: Connection Failed"),
  "Couldn't reach the site. This sign-in is still saved. The uploader will try again on its own.",
);

console.log("upload-lines.test.js ok");

// Rank names and version A copy
{
  const api = require("./upload-lines.js");
  assert.strictEqual(api.humanizeUploadLines(["ranks accepted"]), "Rank names");
  assert.strictEqual(api.sentLine("ranks", 5), "Rank names: 5 ranks");
  assert.strictEqual(api.sentLine("ranks", 1), "Rank names: 1 rank");
  assert.strictEqual(api.sentLine("gbank", 214), "Guild bank: 214 items");
  assert.strictEqual(api.sentLine("recipes", 93), "Recipes: 93 recipes");
  assert.deepStrictEqual(
    api.sentList(
      ["recipes accepted", "gbank accepted", "ranks accepted"],
      [{ ty: "recipes", n: 93 }, { ty: "gbank", n: 214 }, { ty: "ranks", n: 5 }],
    ),
    ["Rank names: 5 ranks", "Guild bank: 214 items", "Recipes: 93 recipes"],
  );
  const parts = api.partitionUploadLines(["ranks Rank names need a Guild Master or officer character linked"]);
  assert.strictEqual(parts.accepted.length, 0);
  const note = api.refusalMessage(parts.refused);
  assert.strictEqual(note.title, "Rank names not sent");
  assert.strictEqual(note.message, "Rank names need a Guild Master or officer character linked. This sign-in is still saved.");
  assert.strictEqual(note.dropsToken, false);
  assert.strictEqual(api.showLastChecked("100", "200"), true);
  assert.strictEqual(api.showLastChecked("200", "200"), false);
  assert.strictEqual(api.showLastChecked(null, "200"), true);
  assert.strictEqual(api.showLastChecked("100", null), false);

  const now = Date.parse("2026-11-10T20:00:00-08:00");
  const loc = "en-US";
  assert.strictEqual(formatUploadTime(null), "Never");
  assert.strictEqual(formatUploadTime(""), "Never");
  // Local wall time, no zone label, no relative phrases. Year only when it is not the current year.
  // 1794356280 / 1794348240 are the mockup instants (4:18 PM and 2:04 PM Pacific on 10 Nov 2026).
  const expectLocal = (raw, withYear) => {
    const d = new Date(Number(raw) * 1000);
    const opts = { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" };
    if (withYear) opts.year = "numeric";
    return d.toLocaleString(loc, opts);
  };
  assert.strictEqual(formatUploadTime("1794356280", now, loc), expectLocal("1794356280", false));
  assert.strictEqual(formatUploadTime("1794348240", now, loc), expectLocal("1794348240", false));
  assert.strictEqual(formatUploadTime("1762820280", now, loc), expectLocal("1762820280", true));
  assert.doesNotMatch(formatUploadTime("1794356280", now, loc), /Just now|ago|Yesterday|\bPST\b|\bGMT\b|\bUTC\b|timeZoneName/);

  assert.strictEqual(formatStatus({ raw: null }), "Not uploaded yet");
  assert.strictEqual(formatStatus({ raw: "Waiting for SavedVariables" }), "Not uploaded yet");
  assert.strictEqual(formatStatus({ sending: true }), "Sending…");
  assert.strictEqual(formatStatus({ failed: true }), "Couldn't send");
  assert.strictEqual(formatStatus({ checkedOnly: true, raw: "sent 2" }), "Nothing new to send");
  assert.strictEqual(formatStatus({ acceptedCount: 3, refusedCount: 0 }), "Sent");
  assert.strictEqual(formatStatus({ acceptedCount: 2, refusedCount: 1 }), "Sent, with an error");
  assert.strictEqual(formatStatus({ raw: "sent 1" }), "Sent");
  assert.strictEqual(formatStatus({ raw: "nothing new" }), "Nothing new to send");
  assert.strictEqual(sentHeading({}), "What was sent");
  assert.strictEqual(sentHeading({ checkedOnly: true }), "Last sent");
  assert.strictEqual(sentHeading({ sending: true }), "Last sent");
  assert.strictEqual(sentHeading({ failed: true }), null);
}
