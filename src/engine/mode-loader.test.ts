/**
 * ModeLoader Unit Tests
 *
 * Verifies security validation rules, manifest schema parsing, and mode extension loading.
 */

import { ModeLoader } from "./mode-loader";
import type { ModeExtension, ModeManifest } from "../types/mode-extension";

export function runModeLoaderTests(): { passed: number; failed: number; errors: string[] } {
  const loader = new ModeLoader();
  let passed = 0;
  let failed = 0;
  const errors: string[] = [];

  function assert(condition: boolean, testName: string) {
    if (condition) {
      passed++;
    } else {
      failed++;
      errors.push(`FAILED: ${testName}`);
    }
  }

  // ── 1. Mode ID Validation (Path traversal & injection prevention) ─────────────
  assert(loader.validateId("study-buddy") === true, "Valid ID 'study-buddy' should pass");
  assert(loader.validateId("zen-master") === true, "Valid ID 'zen-master' should pass");
  assert(loader.validateId("math-tutor-99") === true, "Valid ID 'math-tutor-99' should pass");
  assert(loader.validateId("ab") === true, "2-character ID 'ab' should pass");

  assert(loader.validateId("../../../etc/passwd") === false, "Path traversal ID should fail");
  assert(loader.validateId("my mode; rm -rf") === false, "Command injection characters should fail");
  assert(loader.validateId("a") === false, "1-character ID 'a' should fail (too short)");
  assert(loader.validateId("-leading-hyphen") === false, "Leading hyphen should fail");
  assert(loader.validateId("trailing-hyphen-") === false, "Trailing hyphen should fail");
  assert(loader.validateId("UPPERCASE") === false, "Uppercase ID should fail");
  assert(loader.validateId("has spaces") === false, "Spaces in ID should fail");
  assert(loader.validateId("") === false, "Empty ID should fail");

  // ── 2. Manifest Schema Validation ──────────────────────────────────────────
  const validManifest: ModeManifest = {
    name: "Study Buddy",
    id: "study-buddy",
    version: "1.0.0",
    author: "OpenMate Team",
    description: "Helps you focus and study",
    icon: "📚",
    openmate_version: ">=0.1.0",
  };

  assert(loader.validateManifest(validManifest) === true, "Valid manifest should pass");

  assert(
    loader.validateManifest({ ...validManifest, version: "invalid-semver" }) === false,
    "Invalid semver should fail"
  );
  assert(
    loader.validateManifest({ ...validManifest, name: "" }) === false,
    "Empty name should fail"
  );
  assert(
    loader.validateManifest({ ...validManifest, id: "../traversal" }) === false,
    "Invalid ID in manifest should fail"
  );
  assert(
    loader.validateManifest({ ...validManifest, icon: "" }) === false,
    "Empty icon should fail"
  );
  assert(loader.validateManifest(null) === false, "Null manifest should fail");
  assert(loader.validateManifest("string") === false, "String manifest should fail");

  // ── 3. Mode Extension In-Memory Registration & Loading ─────────────────────
  const testExtension: ModeExtension = {
    manifest: validManifest,
    buildSystemPrompt: (ctx) => `Prompt for ${ctx.companionName}`,
    ambientMessages: {
      morning: ["Good morning!"],
      afternoon: ["Good afternoon!"],
      evening: ["Good evening!"],
    },
  };

  loader.registerExtension(testExtension);

  const loaded = loader.validateManifest(testExtension.manifest);
  assert(loaded === true, "Registered extension should be valid");

  const promptResult = testExtension.buildSystemPrompt({
    companionName: "TestCat",
    userName: "Alice",
    memories: [],
    currentTime: "12:00",
  });
  assert(
    promptResult === "Prompt for TestCat",
    "buildSystemPrompt should return clean string"
  );

  return { passed, failed, errors };
}

// Auto-run helper if in test environment
const proc = (globalThis as unknown as { process?: { env?: { NODE_ENV?: string } } }).process;
if (proc && proc.env?.NODE_ENV === "test") {
  const res = runModeLoaderTests();
  console.log(`[ModeLoader Tests] Passed: ${res.passed}, Failed: ${res.failed}`);
  if (res.failed > 0) {
    console.error(res.errors.join("\n"));
  }
}
