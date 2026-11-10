const assert = require("assert");
const fs = require("fs");
const path = require("path");

const root = path.join(__dirname, "..", "..");
const files = [];
function walk(dir) {
  for (const name of fs.readdirSync(dir)) {
    if (name === "target" || name === "gen") continue;
    const full = path.join(dir, name);
    const stat = fs.statSync(full);
    if (stat.isDirectory()) walk(full);
    else if (name.endsWith(".rs")) files.push(full);
  }
}
walk(root);

const banned = [
  'Command::new("gh")',
  "Command::new(\"gh\")",
  'Command::new("tasklist")',
  'Command::new("pgrep")',
  'Command::new("powershell")',
  'Command::new("curl")',
  'Command::new("unzip")',
  'Command::new("git")',
  "cmd.exe /c",
];

const hits = [];
for (const file of files) {
  const text = fs.readFileSync(file, "utf8");
  for (const needle of banned) {
    if (text.includes(needle)) hits.push(`${path.relative(root, file)} contains ${needle}`);
  }
}
assert.deepStrictEqual(hits, []);

const processRs = fs.readFileSync(path.join(root, "core/src/process.rs"), "utf8");
assert.ok(processRs.includes("creation_flags(CREATE_NO_WINDOW)"));
assert.ok(processRs.includes("0x0800_0000"));
assert.ok(!processRs.includes("Command::new(\"gh\")"));

const updateRs = fs.readFileSync(path.join(root, "core/src/update.rs"), "utf8");
assert.ok(!updateRs.includes("Command::new"));
assert.ok(!updateRs.includes("attest"));

const appUpdate = fs.readFileSync(path.join(root, "app/src-tauri/src/app_update.rs"), "utf8");
assert.ok(appUpdate.includes("background_command"));
assert.ok(!appUpdate.includes("Command::new"));

const libRs = fs.readFileSync(path.join(root, "app/src-tauri/src/lib.rs"), "utf8");
assert.ok(libRs.includes("opener::open"));
assert.ok(!libRs.includes("Command::new"));

console.log("runtime-deps.test.js ok");
