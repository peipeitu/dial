const fs = require("fs");
const path = require("path");

function argValue(name, fallback = "") {
  const prefix = `${name}=`;
  const inline = process.argv.find((arg) => arg.startsWith(prefix));
  if (inline) return inline.slice(prefix.length);
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] || fallback : fallback;
}

function packageSectionVersion(content, fileName) {
  const section = content
    .split(/(?=^\[[^\]]+\]\s*$)/m)
    .find((candidate) => /^\[package\]\s*$/m.test(candidate));
  const version = section?.match(/^version\s*=\s*"([^"]+)"\s*$/m)?.[1];
  if (!version) {
    throw new Error(`Unable to read package version from ${fileName}.`);
  }
  return version;
}

function cargoLockVersion(content) {
  const section = content
    .split(/\n(?=\[\[package\]\]\n)/)
    .find((candidate) => /^name\s*=\s*"dial"\s*$/m.test(candidate));
  const version = section?.match(/^version\s*=\s*"([^"]+)"\s*$/m)?.[1];
  if (!version) {
    throw new Error("Unable to read Dial version from src-tauri/Cargo.lock.");
  }
  return version;
}

function parseSemanticVersion(value) {
  const match = value.match(
    /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/,
  );
  if (!match) {
    throw new Error(`Invalid semantic version: ${value}.`);
  }
  return {
    core: match.slice(1, 4).map(BigInt),
    prerelease: match[4] ? match[4].split(".") : [],
  };
}

function comparePrerelease(left, right) {
  if (!left.length && !right.length) return 0;
  if (!left.length) return 1;
  if (!right.length) return -1;

  const length = Math.max(left.length, right.length);
  for (let index = 0; index < length; index += 1) {
    if (left[index] === undefined) return -1;
    if (right[index] === undefined) return 1;
    if (left[index] === right[index]) continue;

    const leftNumeric = /^\d+$/.test(left[index]);
    const rightNumeric = /^\d+$/.test(right[index]);
    if (leftNumeric && rightNumeric) {
      return BigInt(left[index]) < BigInt(right[index]) ? -1 : 1;
    }
    if (leftNumeric !== rightNumeric) return leftNumeric ? -1 : 1;
    return left[index] < right[index] ? -1 : 1;
  }
  return 0;
}

function compareSemanticVersions(leftValue, rightValue) {
  const left = parseSemanticVersion(leftValue);
  const right = parseSemanticVersion(rightValue);
  for (let index = 0; index < left.core.length; index += 1) {
    if (left.core[index] !== right.core[index]) {
      return left.core[index] < right.core[index] ? -1 : 1;
    }
  }
  return comparePrerelease(left.prerelease, right.prerelease);
}

function assertReleaseNotOlder(tag, latestTag) {
  if (!tag || !latestTag) return;
  const comparison = compareSemanticVersions(tag, latestTag);
  if (comparison < 0) {
    throw new Error(`Release tag ${tag} is older than latest release ${latestTag}.`);
  }
  if (comparison === 0 && tag.replace(/^v/, "") !== latestTag.replace(/^v/, "")) {
    throw new Error(`Release tag ${tag} conflicts with latest release ${latestTag}.`);
  }
}

function assertReleaseIsLatest(tag, latestTag) {
  if (!tag || !latestTag || tag !== latestTag) {
    throw new Error(
      `Release repair target ${tag || "<missing>"} must match latest release ${latestTag || "<missing>"}.`,
    );
  }
}

function releaseVersions(root) {
  const packageJson = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
  const packageLock = JSON.parse(fs.readFileSync(path.join(root, "package-lock.json"), "utf8"));
  const cargoToml = fs.readFileSync(path.join(root, "src-tauri", "Cargo.toml"), "utf8");
  const cargoLock = fs.readFileSync(path.join(root, "src-tauri", "Cargo.lock"), "utf8");
  const tauriConfig = JSON.parse(
    fs.readFileSync(path.join(root, "src-tauri", "tauri.conf.json"), "utf8"),
  );

  return {
    "package.json": packageJson.version,
    "package-lock.json": packageLock.version,
    "package-lock.json packages[\"\"]": packageLock.packages?.[""]?.version,
    "src-tauri/Cargo.toml": packageSectionVersion(cargoToml, "src-tauri/Cargo.toml"),
    "src-tauri/Cargo.lock": cargoLockVersion(cargoLock),
    "src-tauri/tauri.conf.json": tauriConfig.version,
  };
}

function checkRelease(root, tag = "", latestTag = "", requireLatest = false) {
  const versions = releaseVersions(root);
  const entries = Object.entries(versions);
  const missing = entries.filter(([, version]) => !version).map(([file]) => file);
  if (missing.length) {
    throw new Error(`Missing version in ${missing.join(", ")}.`);
  }

  const expectedVersion = versions["package.json"];
  const mismatches = entries.filter(([, version]) => version !== expectedVersion);
  if (mismatches.length) {
    const details = entries.map(([file, version]) => `${file}=${version}`).join(", ");
    throw new Error(`Release versions are not synchronized: ${details}.`);
  }

  const changelog = fs.readFileSync(path.join(root, "CHANGELOG.md"), "utf8");
  if (!changelog.includes(`## ${expectedVersion} -`)) {
    throw new Error(`CHANGELOG.md has no release entry for ${expectedVersion}.`);
  }

  if (tag && tag !== `v${expectedVersion}`) {
    throw new Error(`Release tag ${tag} does not match version v${expectedVersion}.`);
  }
  assertReleaseNotOlder(tag, latestTag);
  if (requireLatest) {
    assertReleaseIsLatest(tag, latestTag);
  }

  return expectedVersion;
}

function main() {
  const root = path.resolve(argValue("--root", path.join(__dirname, "..")));
  const tag = argValue("--tag", process.env.RELEASE_TAG || "").trim();
  const latestTag = argValue("--latest-tag", process.env.LATEST_RELEASE_TAG || "").trim();
  const requireLatest = process.argv.includes("--require-latest");
  const version = checkRelease(root, tag, latestTag, requireLatest);
  console.log(`Release configuration is synchronized at ${version}${tag ? ` (${tag})` : ""}.`);
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

module.exports = {
  assertReleaseIsLatest,
  assertReleaseNotOlder,
  checkRelease,
  compareSemanticVersions,
  releaseVersions,
};
