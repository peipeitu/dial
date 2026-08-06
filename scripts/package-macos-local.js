const path = require("node:path");
const { spawnSync } = require("node:child_process");

const root = path.resolve(__dirname, "..");
const tauri = path.join(root, "node_modules", ".bin", "tauri");
const appBundle = path.join(
  root,
  "src-tauri",
  "target",
  "release",
  "bundle",
  "macos",
  "Dial.app"
);

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: root,
    stdio: "inherit"
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

if (process.platform !== "darwin") {
  console.error("Local macOS packaging must run on macOS.");
  process.exit(1);
}

run(tauri, ["build", "--bundles", "app"]);
run("codesign", ["--force", "--deep", "--sign", "-", appBundle]);
run("codesign", ["--verify", "--deep", "--strict", appBundle]);

console.log(`Built and verified local app bundle: ${appBundle}`);
