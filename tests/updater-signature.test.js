const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const test = require("node:test");

const { verifyUpdaterSignature } = require("../scripts/verify-updater-signatures.js");

const root = path.resolve(__dirname, "..");
const script = path.join(root, "scripts", "verify-updater-signatures.js");
const publicKeyText = `untrusted comment: minisign public key E7620F1842B4E81F
RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3`;
const signatureText = `untrusted comment: signature from minisign secret key
RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=
trusted comment: timestamp:1556193335\tfile:test
y/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg==`;
const encodedPublicKey = Buffer.from(publicKeyText).toString("base64");
const encodedSignature = Buffer.from(signatureText).toString("base64");

test("updater signature verification accepts a matching Tauri key and signature", () => {
  assert.doesNotThrow(() =>
    verifyUpdaterSignature(Buffer.from("test"), encodedSignature, encodedPublicKey),
  );
});

test("updater signature verification rejects modified artifact contents", () => {
  assert.throws(
    () => verifyUpdaterSignature(Buffer.from("modified"), encodedSignature, encodedPublicKey),
    /signature verification failed/,
  );
});

test("updater signature verification rejects a different public key", () => {
  const lines = publicKeyText.split("\n");
  const packet = Buffer.from(lines[1], "base64");
  packet[2] ^= 0xff;
  const differentPublicKey = Buffer.from(`${lines[0]}\n${packet.toString("base64")}`).toString(
    "base64",
  );

  assert.throws(
    () => verifyUpdaterSignature(Buffer.from("test"), encodedSignature, differentPublicKey),
    /created by a different key/,
  );
});

test("artifact verification checks both release platforms", (t) => {
  const artifactsDir = fs.mkdtempSync(path.join(os.tmpdir(), "dial-signatures-"));
  t.after(() => fs.rmSync(artifactsDir, { recursive: true, force: true }));

  for (const name of ["Dial_1.2.3_x64-setup.exe", "Dial_1.2.3_amd64.AppImage"]) {
    const artifact = path.join(artifactsDir, name);
    fs.writeFileSync(artifact, "test");
    fs.writeFileSync(`${artifact}.sig`, encodedSignature);
  }

  const result = spawnSync(
    process.execPath,
    [script, "--artifacts", artifactsDir],
    {
      encoding: "utf8",
      env: { ...process.env, AI_USAGE_UPDATER_PUBLIC_KEY: encodedPublicKey },
    },
  );

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Windows|x64-setup\.exe/);
  assert.match(result.stdout, /AppImage/);
});
