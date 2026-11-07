const assert = require("assert");
const fs = require("fs");
const path = require("path");
const { connectPhase } = require("./connect-recovery.js");

const waiting = connectPhase("waiting");
const cancelled = connectPhase("cancelled");

assert.strictEqual(waiting.status, "Waiting for the browser.");
assert.strictEqual(waiting.cancelHidden, false);
assert.strictEqual(waiting.switchHidden, true);
assert.match(waiting.recovery, /Cancel here first/);
assert.match(waiting.recovery, /this app has not received that/);
assert.doesNotMatch(waiting.recovery, /right role/);

assert.strictEqual(cancelled.status, "Sign-in didn't finish.");
assert.strictEqual(cancelled.saved, "Nothing was saved.");
assert.strictEqual(cancelled.cancelHidden, true);
assert.strictEqual(cancelled.switchHidden, false);
assert.strictEqual(cancelled.cta, "Sign in with a different account");
assert.strictEqual(
  cancelled.recovery,
  "Sign-in was cancelled. To use another account, switch accounts or log out of Discord in your browser, then choose “Sign in with a different account” below. Signing out of the guild website does not switch Discord accounts."
);
assert.doesNotMatch(cancelled.recovery, /Cancel here/);
assert.doesNotMatch(cancelled.recovery, /\bCancel\b/);
assert.doesNotMatch(cancelled.recovery, /right role|received a decline|denial callback/i);

const html = fs.readFileSync(path.join(__dirname, "index.html"), "utf8");
assert.match(html, /Sign in with a different account/);
assert.match(html, /id="btn-cancel-connect"/);
assert.doesNotMatch(html, /Sign-in was cancelled/);

const app = fs.readFileSync(path.join(__dirname, "app.js"), "utf8");
assert.match(app, /showRecovery\("waiting"\)/);
assert.match(app, /showRecovery\("cancelled"\)/);

console.log("connect-recovery.test.js ok");
