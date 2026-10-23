// Fails when any UI script cannot be parsed as an ES module. The windows load these files
// directly, so one syntax error stops every script that imports it (the taskbar widget then
// shows the flyout's header instead of the usage rings).
const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");
const os = require("os");

const dir = path.join(__dirname, "..", "src");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "slopbar-ui-"));
let failed = 0;
for (const file of fs.readdirSync(dir).filter((f) => f.endsWith(".js"))) {
  // node --check needs a .mjs name to parse a file as a module.
  const copy = path.join(tmp, file.replace(/\.js$/, ".mjs"));
  fs.copyFileSync(path.join(dir, file), copy);
  try {
    execFileSync(process.execPath, ["--check", copy], { stdio: "pipe" });
  } catch (err) {
    failed++;
    console.error(`src/${file}: ${String(err.stderr).trim().split("\n").slice(0, 4).join("\n")}`);
  }
}
fs.rmSync(tmp, { recursive: true, force: true });
if (failed) {
  console.error(`${failed} UI script(s) do not parse`);
  process.exit(1);
}
console.log("all UI scripts parse");
