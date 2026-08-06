const assert = require("node:assert/strict");
const test = require("node:test");

const { validateManifest, verifyUpdaterRelease } = require("../scripts/verify-updater-release.js");

const publicKeyText = `untrusted comment: minisign public key E7620F1842B4E81F
RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3`;
const signatureText = `untrusted comment: signature from minisign secret key
RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=
trusted comment: timestamp:1556193335\tfile:test
y/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg==`;
const encodedPublicKey = Buffer.from(publicKeyText).toString("base64");
const encodedSignature = Buffer.from(signatureText).toString("base64");
const signedArtifact = Buffer.from("test");

function manifestFixture() {
  return {
    version: "1.2.3",
    platforms: {
      "windows-x86_64": {
        signature: encodedSignature,
        url: "https://github.com/owner/repo/releases/download/v1.2.3/Dial_1.2.3_x64-setup.exe",
      },
      "linux-x86_64": {
        signature: encodedSignature,
        url: "https://github.com/owner/repo/releases/download/v1.2.3/Dial_1.2.3_amd64.AppImage",
      },
    },
  };
}

function mockResponse({ status = 200, json, text = "", data = Buffer.alloc(0) } = {}) {
  const buffer = Buffer.from(data);
  return {
    ok: status >= 200 && status < 300,
    status,
    async json() {
      return json;
    },
    async text() {
      return text;
    },
    async arrayBuffer() {
      return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
    },
  };
}

test("manifest validation rejects asset names with whitespace", () => {
  const manifest = manifestFixture();
  manifest.platforms["windows-x86_64"].url =
    "https://github.com/owner/repo/releases/download/v1.2.3/Dial%20Setup_1.2.3_x64.exe";

  assert.throws(
    () => validateManifest(manifest, "owner/repo", "v1.2.3"),
    /contains whitespace/,
  );
});

test("manifest validation accepts SemVer build metadata in the release tag", () => {
  const manifest = manifestFixture();
  manifest.version = "1.2.3+build.1";
  for (const entry of Object.values(manifest.platforms)) {
    entry.url = entry.url.replace("v1.2.3", "v1.2.3+build.1");
  }

  assert.doesNotThrow(() =>
    validateManifest(manifest, "owner/repo", "v1.2.3+build.1"),
  );
});

test("manifest validation rejects extra path segments and URL parameters", () => {
  const extraSegment = manifestFixture();
  extraSegment.platforms["windows-x86_64"].url =
    "https://github.com/owner/repo/releases/download/v1.2.3/nested/app.exe";
  assert.throws(
    () => validateManifest(extraSegment, "owner/repo", "v1.2.3"),
    /does not target/,
  );

  const queryString = manifestFixture();
  queryString.platforms["windows-x86_64"].url += "?download=1";
  assert.throws(
    () => validateManifest(queryString, "owner/repo", "v1.2.3"),
    /does not target/,
  );
});

test("release verification downloads and cryptographically verifies released assets", async () => {
  const manifest = manifestFixture();
  const requested = [];
  const fetchImpl = async (url, init = {}) => {
    requested.push([url, init.method || "GET"]);
    assert.ok(init.signal instanceof AbortSignal);
    if (url.endsWith("/latest.json")) {
      return mockResponse({ json: manifest });
    }
    if (url.endsWith(".exe.sig")) {
      return mockResponse({ text: `${encodedSignature}\n` });
    }
    if (url.endsWith(".AppImage.sig")) {
      return mockResponse({ text: `${encodedSignature}\n` });
    }
    if (Object.values(manifest.platforms).some((entry) => entry.url === url)) {
      return mockResponse({ data: signedArtifact });
    }
    throw new Error(`Unexpected URL: ${url}`);
  };

  await verifyUpdaterRelease({
    repo: "owner/repo",
    tag: "v1.2.3",
    encodedPublicKey,
    attempts: 1,
    delayMs: 0,
    timeoutMs: 1000,
    fetchImpl,
    log() {},
  });

  assert.deepEqual(
    requested
      .filter(([url]) => Object.values(manifest.platforms).some((entry) => entry.url === url))
      .map(([url]) => url),
    [
      manifest.platforms["windows-x86_64"].url,
      manifest.platforms["linux-x86_64"].url,
    ],
  );
  assert.equal(requested.some(([, method]) => method === "HEAD"), false);
  assert.deepEqual(
    requested.filter(([url]) => url.endsWith("/latest.json")).map(([url]) => url),
    [
      "https://github.com/owner/repo/releases/download/v1.2.3/latest.json",
      "https://github.com/owner/repo/releases/latest/download/latest.json",
    ],
  );
});

test("release verification rejects a signature mismatch", async () => {
  const manifest = manifestFixture();
  const fetchImpl = async (url, init = {}) => {
    if (url.endsWith("/latest.json")) return mockResponse({ json: manifest });
    if (url.endsWith(".sig")) return mockResponse({ text: "wrong signature" });
    return mockResponse({ data: signedArtifact });
  };

  await assert.rejects(
    verifyUpdaterRelease({
      repo: "owner/repo",
      tag: "v1.2.3",
      encodedPublicKey,
      attempts: 1,
      delayMs: 0,
      timeoutMs: 1000,
      fetchImpl,
      log() {},
    }),
    /signature does not match/,
  );
});

test("release verification retries content mismatches and refetches manifests", async () => {
  const manifest = manifestFixture();
  let taggedManifestReads = 0;
  let windowsSignatureReads = 0;
  const fetchImpl = async (url, init = {}) => {
    if (url.includes("/releases/download/") && url.endsWith("/latest.json")) {
      taggedManifestReads += 1;
      return mockResponse({ json: manifest });
    }
    if (url.includes("/releases/latest/") && url.endsWith("/latest.json")) {
      return mockResponse({ json: manifest });
    }
    if (url.endsWith(".exe.sig")) {
      windowsSignatureReads += 1;
      return mockResponse({
        text: windowsSignatureReads === 1 ? "stale signature" : encodedSignature,
      });
    }
    if (url.endsWith(".AppImage.sig")) return mockResponse({ text: encodedSignature });
    return mockResponse({ data: signedArtifact });
  };

  await verifyUpdaterRelease({
    repo: "owner/repo",
    tag: "v1.2.3",
    encodedPublicKey,
    attempts: 2,
    delayMs: 0,
    timeoutMs: 1000,
    fetchImpl,
    log() {},
  });

  assert.equal(windowsSignatureReads, 2);
  assert.equal(taggedManifestReads, 2);
});

test("release verification rejects a latest alias that points to another version", async () => {
  const manifest = manifestFixture();
  const staleManifest = manifestFixture();
  staleManifest.version = "1.2.2";
  const fetchImpl = async (url) => {
    if (url.includes("/releases/latest/")) return mockResponse({ json: staleManifest });
    return mockResponse({ json: manifest });
  };

  await assert.rejects(
    verifyUpdaterRelease({
      repo: "owner/repo",
      tag: "v1.2.3",
      encodedPublicKey,
      attempts: 1,
      delayMs: 0,
      timeoutMs: 1000,
      fetchImpl,
      log() {},
    }),
    /does not match 1\.2\.3/,
  );
});

test("release verification rejects a released artifact that does not match its signature", async () => {
  const manifest = manifestFixture();
  const fetchImpl = async (url) => {
    if (url.endsWith("/latest.json")) return mockResponse({ json: manifest });
    if (url.endsWith(".sig")) return mockResponse({ text: encodedSignature });
    return mockResponse({ data: Buffer.from("tampered") });
  };

  await assert.rejects(
    verifyUpdaterRelease({
      repo: "owner/repo",
      tag: "v1.2.3",
      encodedPublicKey,
      attempts: 1,
      delayMs: 0,
      timeoutMs: 1000,
      fetchImpl,
      log() {},
    }),
    /released artifact verification failed/,
  );
});
