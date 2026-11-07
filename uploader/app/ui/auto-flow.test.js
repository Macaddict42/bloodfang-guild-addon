const assert = require("assert");
const fs = require("fs");
const path = require("path");

const html = fs.readFileSync(path.join(__dirname, "index.html"), "utf8");
const js = fs.readFileSync(path.join(__dirname, "app.js"), "utf8");

assert.ok(!html.includes('id="btn-upload"'));
assert.ok(!html.includes("Check for update"));
assert.ok(!html.includes('id="btn-addon-update"'));
assert.ok(html.includes('id="btn-app-update"'));
assert.ok(html.includes('id="btn-app-later"'));
assert.ok(js.includes("start_watch"));
assert.ok(js.includes("check_self_update"));
assert.ok(js.includes("defer_app_update"));
assert.ok(js.includes("accept_app_update"));
assert.ok(!js.includes('invoke("upload_now")'));
assert.ok(js.includes("update is available. Update the app now?"));
assert.ok(!js.includes("A Bloodfang App update"));
assert.ok(js.includes("querySelector(\"h1\")"));

console.log("auto-flow.test.js ok");
