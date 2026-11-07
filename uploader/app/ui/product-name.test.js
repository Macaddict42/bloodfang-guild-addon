const assert = require("assert");
const fs = require("fs");
const path = require("path");

const html = fs.readFileSync(path.join(__dirname, "index.html"), "utf8");
const conf = JSON.parse(fs.readFileSync(path.join(__dirname, "../src-tauri/tauri.conf.json"), "utf8"));

assert.ok(conf.productName);
assert.strictEqual(conf.identifier, "com.raidgroup.guilduploader");
assert.ok(conf.bundle.publisher);
assert.strictEqual(conf.app.windows[0].title, conf.productName);
assert.match(html, new RegExp(`<title>${conf.productName}</title>`));
assert.match(html, new RegExp(`<h1>${conf.productName}</h1>`));
assert.strictEqual(conf.app.trayIcon, undefined);
assert.strictEqual(conf.bundle.windows.nsis.installerHooks, "windows/hooks.nsh");
assert.strictEqual(conf.productName, "Bloodfang App");
assert.strictEqual(conf.bundle.publisher, "Bloodfang Guild");
assert.ok(!html.includes("Guild Uploader"));
assert.ok(!html.includes("Upload now"));
assert.ok(!html.includes("Update available"));
assert.ok(!html.includes("A new update is available. Would you like to update now?"));
assert.ok(html.includes("Saved scans upload on their own."));

console.log("product-name.test.js ok");
