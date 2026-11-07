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
assert.ok(!html.includes("Guild Uploader"));

console.log("product-name.test.js ok");
