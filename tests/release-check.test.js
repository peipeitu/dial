const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const test = require("node:test");

const { compareSemanticVersions } = require("../scripts/check-release.js");

const root = path.resolve(__dirname, "..");
const script = path.join(root, "scripts", "check-release.js");

function releaseFixture(t, versions = {}) {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "dial-release-check-"));
  t.after(() => fs.rmSync(fixture, { recursive: true, force: true }));
  fs.mkdirSync(path.join(fixture, "src-tauri"), { recursive: true });

  const version = versions.packageJson || "1.2.3";
  fs.writeFileSync(
    path.join(fixture, "package.json"),
    JSON.stringify({ name: "dial", version }),
  );
  fs.writeFileSync(
    path.join(fixture, "package-lock.json"),
    JSON.stringify({
      version: versions.packageLock || version,
      packages: { "": { version: versions.packageLockRoot || version } },
    }),
  );
  fs.writeFileSync(
    path.join(fixture, "src-tauri", "Cargo.toml"),
    `[package]\nname = "dial"\nversion = "${versions.cargoToml || version}"\n\n[dependencies]\n`,
  );
  fs.writeFileSync(
    path.join(fixture, "src-tauri", "Cargo.lock"),
    `version = 4\n\n[[package]]\nname = "dial"\nversion = "${versions.cargoLock || version}"\n`,
  );
  fs.writeFileSync(
    path.join(fixture, "src-tauri", "tauri.conf.json"),
    JSON.stringify({ version: versions.tauri || version }),
  );
  fs.writeFileSync(path.join(fixture, "CHANGELOG.md"), `# Changelog\n\n## ${version} - 2026-07-16\n`);
  return fixture;
}

function runCheck(fixture, tag, latestTag = "", requireLatest = false) {
  const args = [
    script,
    "--root",
    fixture,
    "--tag",
    tag,
    "--latest-tag",
    latestTag,
  ];
  if (requireLatest) args.push("--require-latest");
  return spawnSync(process.execPath, args, {
    encoding: "utf8",
  });
}

test("release check accepts synchronized versions and matching tag", (t) => {
  const result = runCheck(releaseFixture(t), "v1.2.3");

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /synchronized at 1\.2\.3 \(v1\.2\.3\)/);
});

test("release check rejects mismatched metadata", (t) => {
  const result = runCheck(releaseFixture(t, { tauri: "1.2.4" }), "v1.2.3");

  assert.equal(result.status, 1);
  assert.match(result.stderr, /versions are not synchronized/);
});

test("release check rejects a tag that does not match the app version", (t) => {
  const result = runCheck(releaseFixture(t), "v1.2.4");

  assert.equal(result.status, 1);
  assert.match(result.stderr, /does not match version v1\.2\.3/);
});

test("release check rejects a tag older than the current latest release", (t) => {
  const result = runCheck(releaseFixture(t), "v1.2.3", "v1.3.0");

  assert.equal(result.status, 1);
  assert.match(result.stderr, /older than latest release v1\.3\.0/);
});

test("release check accepts a tag newer than the current latest release", (t) => {
  const result = runCheck(releaseFixture(t), "v1.2.3", "v1.2.2");

  assert.equal(result.status, 0, result.stderr);
});

test("release repair rejects a published tag newer than the current latest release", (t) => {
  const result = runCheck(releaseFixture(t), "v1.2.3", "v1.2.2", true);

  assert.equal(result.status, 1);
  assert.match(result.stderr, /must match latest release v1\.2\.2/);
});

test("release repair accepts the exact current latest tag", (t) => {
  const result = runCheck(releaseFixture(t), "v1.2.3", "v1.2.3", true);

  assert.equal(result.status, 0, result.stderr);
});

test("release check rejects a different tag with equal semantic precedence", (t) => {
  const fixture = releaseFixture(t, { packageJson: "1.2.3+repair" });
  const result = runCheck(fixture, "v1.2.3+repair", "v1.2.3+release");

  assert.equal(result.status, 1);
  assert.match(result.stderr, /conflicts with latest release/);
});

test("release check rejects an older arbitrarily large numeric prerelease", (t) => {
  const version = "1.2.3-alpha.9007199254740992";
  const fixture = releaseFixture(t, { packageJson: version });
  const result = runCheck(
    fixture,
    `v${version}`,
    "v1.2.3-alpha.9007199254740993",
  );

  assert.equal(result.status, 1);
  assert.match(result.stderr, /older than latest release/);
});

test("semantic version comparison orders prereleases before stable versions", () => {
  assert.equal(compareSemanticVersions("v1.2.3-beta.2", "v1.2.3-beta.10"), -1);
  assert.equal(compareSemanticVersions("v1.2.3-rc.1", "v1.2.3"), -1);
  assert.equal(compareSemanticVersions("v1.2.3", "v1.2.3"), 0);
});

test("semantic version comparison preserves arbitrarily large numeric identifiers", () => {
  assert.equal(
    compareSemanticVersions(
      "v1.2.3-alpha.9007199254740992",
      "v1.2.3-alpha.9007199254740993",
    ),
    -1,
  );
  assert.equal(
    compareSemanticVersions("v9007199254740992.0.0", "v9007199254740993.0.0"),
    -1,
  );
});
