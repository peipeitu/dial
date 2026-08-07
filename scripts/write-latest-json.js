const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const packageJson = require(path.join(root, "package.json"));

function argValue(name, fallback = "") {
  const prefix = `${name}=`;
  const inline = process.argv.find((arg) => arg.startsWith(prefix));
  if (inline) return inline.slice(prefix.length);
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] || fallback : fallback;
}

function walkFiles(dir) {
  if (!fs.existsSync(dir)) return [];
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  return entries.flatMap((entry) => {
    const fullPath = path.join(dir, entry.name);
    return entry.isDirectory() ? walkFiles(fullPath) : [fullPath];
  });
}

function findOne(files, predicate, label) {
  const matches = files.filter(predicate);
  if (matches.length !== 1) {
    throw new Error(`Expected exactly one ${label}, found ${matches.length}.`);
  }
  return matches[0];
}

function signatureFor(file) {
  const signaturePath = `${file}.sig`;
  if (!fs.existsSync(signaturePath)) {
    throw new Error(`Missing updater signature for ${file}.`);
  }
  return fs.readFileSync(signaturePath, "utf8").trim();
}

function releaseAssetName(file) {
  // GitHub replaces whitespace with periods when it stores release asset names.
  return path.basename(file).replace(/\s+/g, ".");
}

function assetUrl(repo, tag, file) {
  return `https://github.com/${repo}/releases/download/${tag}/${encodeURIComponent(releaseAssetName(file))}`;
}

const artifactsDir = path.resolve(argValue("--artifacts", path.join(root, "release-artifacts")));
const outputPath = path.resolve(argValue("--output", path.join(artifactsDir, "latest.json")));
const repo = argValue("--repo", process.env.GITHUB_REPOSITORY || "peipeitu/dial");
const tag = argValue("--tag", process.env.GITHUB_REF_NAME || `v${packageJson.version}`);
const version = tag.replace(/^v/, "") || packageJson.version;
const files = walkFiles(artifactsDir);

const windowsInstaller = findOne(
  files,
  (file) => file.endsWith(".exe"),
  "Windows NSIS installer",
);
const linuxAppImage = findOne(
  files,
  (file) => file.endsWith(".AppImage"),
  "Linux AppImage",
);

// Older clients re-stringify pub_date into a value WebView2 cannot parse and fail to open the update dialog.
const latest = {
  version,
  notes: `Dial ${version}`,
  platforms: {
    "windows-x86_64": {
      signature: signatureFor(windowsInstaller),
      url: assetUrl(repo, tag, windowsInstaller),
    },
    "linux-x86_64": {
      signature: signatureFor(linuxAppImage),
      url: assetUrl(repo, tag, linuxAppImage),
    },
  },
};

fs.mkdirSync(path.dirname(outputPath), { recursive: true });
fs.writeFileSync(outputPath, `${JSON.stringify(latest, null, 2)}\n`);
console.log(`Wrote updater manifest to ${outputPath}`);
