const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const test = require("node:test");

const root = path.resolve(__dirname, "..");
const script = path.join(root, "scripts", "write-latest-json.js");

function artifactFixture(t) {
  const artifactsDir = fs.mkdtempSync(path.join(os.tmpdir(), "dial-updater-"));
  t.after(() => fs.rmSync(artifactsDir, { recursive: true, force: true }));

  const windowsInstaller = path.join(artifactsDir, "Dial_1.2.3_x64-setup.exe");
  const linuxAppImage = path.join(artifactsDir, "Dial_1.2.3_amd64.AppImage");

  fs.writeFileSync(windowsInstaller, "windows installer");
  fs.writeFileSync(`${windowsInstaller}.sig`, "windows signature");
  fs.writeFileSync(linuxAppImage, "linux appimage");
  fs.writeFileSync(`${linuxAppImage}.sig`, "linux signature");

  return { artifactsDir, linuxAppImage, windowsInstaller };
}

function runManifest(artifactsDir) {
  const outputPath = path.join(artifactsDir, "latest.json");
  const result = spawnSync(
    process.execPath,
    [
      script,
      "--artifacts",
      artifactsDir,
      "--output",
      outputPath,
      "--repo",
      "owner/repo",
      "--tag",
      "v1.2.3",
    ],
    { encoding: "utf8" },
  );
  return { outputPath, result };
}

test("updater manifest uses GitHub's normalized release asset names", (t) => {
  const { artifactsDir } = artifactFixture(t);
  const { outputPath, result } = runManifest(artifactsDir);

  assert.equal(result.status, 0, result.stderr);
  const manifest = JSON.parse(fs.readFileSync(outputPath, "utf8"));

  assert.equal(
    manifest.platforms["windows-x86_64"].url,
    "https://github.com/owner/repo/releases/download/v1.2.3/Dial_1.2.3_x64-setup.exe",
  );
  assert.equal(
    manifest.platforms["linux-x86_64"].url,
    "https://github.com/owner/repo/releases/download/v1.2.3/Dial_1.2.3_amd64.AppImage",
  );
  assert.equal(manifest.platforms["windows-x86_64"].signature, "windows signature");
  assert.equal(manifest.platforms["linux-x86_64"].signature, "linux signature");
  assert.equal("pub_date" in manifest, false);
});

test("updater manifest rejects a missing signature", (t) => {
  const { artifactsDir, linuxAppImage } = artifactFixture(t);
  fs.rmSync(`${linuxAppImage}.sig`);

  const { result } = runManifest(artifactsDir);

  assert.equal(result.status, 1);
  assert.match(result.stderr, /Missing updater signature/);
});

test("updater manifest rejects duplicate platform installers", (t) => {
  const { artifactsDir } = artifactFixture(t);
  fs.writeFileSync(path.join(artifactsDir, "another-setup.exe"), "duplicate installer");

  const { result } = runManifest(artifactsDir);

  assert.equal(result.status, 1);
  assert.match(result.stderr, /Expected exactly one Windows NSIS installer, found 2/);
});
