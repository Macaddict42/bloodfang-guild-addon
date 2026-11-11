const assert = require("assert");
const fs = require("fs");
const path = require("path");

const js = fs.readFileSync(path.join(__dirname, "app.js"), "utf8");

// A failed addon check shows the app's reason, worded as a check (not an update) failure.
assert.ok(js.includes('setAddonUi("check-failed", lastAddonCheck);'));
assert.ok(js.includes('showErr("addon-err", "addon-err-msg", e, "Addon check failed")'));
assert.ok(js.includes('r.phase === "check"'));
assert.ok(js.includes('"Couldn\'t check for an addon update."'));
// The upload failure line shows the app's message as sent (no extra prefix in the window).
assert.ok(js.includes('showErr("upload-err", "upload-err-msg", msg, "Upload failed")'));

console.log("error-reasons.test.js ok");
