const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const { releaseMutationPolicy } = require("../scripts/release-policy.js");

test("release state probe preserves the false default when no release exists", () => {
  const workflow = fs.readFileSync(
    path.join(__dirname, "..", ".github", "workflows", "build.yml"),
    "utf8",
  );
  const inspectReleaseState = workflow
    .split("- name: Inspect release state")[1]
    .split("- name: Verify existing published release")[0];

  assert.doesNotMatch(
    inspectReleaseState,
    /if release_is_draft=\$\(gh release view "\$RELEASE_TAG"/,
  );
  assert.match(
    inspectReleaseState,
    /if queried_release_is_draft=\$\(gh release view "\$RELEASE_TAG"[\s\S]*release_is_draft="\$queried_release_is_draft"/,
  );
});

test("preflight defers an invisible repair target to the write-capable release job", () => {
  const workflow = fs.readFileSync(
    path.join(__dirname, "..", ".github", "workflows", "build.yml"),
    "utf8",
  );
  const validateRepairTarget = workflow
    .split("- name: Validate release ordering and repair target")[1]
    .split("- name: Require updater signing for tagged releases")[0];

  assert.doesNotMatch(
    validateRepairTarget,
    /::error::Release repair requires an existing release/,
  );
  assert.match(
    validateRepairTarget,
    /Repair target visibility and existence will be validated by the Release job/,
  );
});

test("release policy creates a missing release", () => {
  assert.deepEqual(
    releaseMutationPolicy({ exists: false, isDraft: false, repair: false }),
    { shouldMutate: true },
  );
});

test("release policy resumes an existing draft", () => {
  assert.deepEqual(
    releaseMutationPolicy({ exists: true, isDraft: true, repair: false }),
    { shouldMutate: true },
  );
});

test("release policy keeps a published release read-only by default", () => {
  assert.deepEqual(
    releaseMutationPolicy({ exists: true, isDraft: false, repair: false }),
    { shouldMutate: false },
  );
});

test("release policy permits an explicit published-release repair", () => {
  assert.deepEqual(
    releaseMutationPolicy({ exists: true, isDraft: false, repair: true }),
    { shouldMutate: true },
  );
});

test("release policy resumes a draft during explicit repair", () => {
  assert.deepEqual(
    releaseMutationPolicy({ exists: true, isDraft: true, repair: true }),
    { shouldMutate: true },
  );
});

test("release policy rejects repair for a missing release", () => {
  assert.throws(
    () => releaseMutationPolicy({ exists: false, isDraft: false, repair: true }),
    /existing release/,
  );
});
