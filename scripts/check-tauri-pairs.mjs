#!/usr/bin/env node
/**
 * Fail when a Tauri Rust crate and its npm counterpart disagree on major.minor.
 *
 * `tauri build` refuses to run on such a mismatch ("Found version mismatched
 * Tauri packages"), but nothing in the PR gate runs `tauri build` — so the
 * mismatch sails through CI and only surfaces when a release tag is pushed and
 * all three platform builds fail at once. That is exactly what happened to
 * v0.11.0: a grouped cargo bump moved `tauri-plugin-updater` to 2.11.0 while
 * `@tauri-apps/plugin-updater` stayed on 2.10.1, because the two live in
 * different ecosystems and Dependabot updates them independently.
 *
 * This is a pure version comparison — no build, no network, ~50ms.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

/** Every `name`/`version` pair in Cargo.lock, keyed by crate name. */
function readCargoLock() {
  const text = readFileSync(join(root, "src-tauri/Cargo.lock"), "utf8");
  const versions = new Map();
  const re = /^name = "([^"]+)"\nversion = "([^"]+)"/gm;
  for (const m of text.matchAll(re)) versions.set(m[1], m[2]);
  return versions;
}

/** Resolved npm versions, preferring the lockfile over the manifest range. */
function readNpmVersions() {
  const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
  const declared = { ...pkg.dependencies, ...pkg.devDependencies };
  const lock = readFileSync(join(root, "pnpm-lock.yaml"), "utf8");
  const out = new Map();
  for (const name of Object.keys(declared)) {
    if (!name.startsWith("@tauri-apps/")) continue;
    // pnpm-lock keys look like `'@tauri-apps/plugin-updater@2.11.0':`
    const esc = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    // Collect every resolved version, not just the first match: a lockfile can
    // legitimately carry two versions of one package, and picking whichever
    // appears first would silently check the wrong one.
    const found = [
      ...new Set(
        [...lock.matchAll(new RegExp(`${esc}@(\\d+\\.\\d+\\.\\d+)`, "g"))].map((m) => m[1]),
      ),
    ];
    out.set(name, found.length > 0 ? found : [declared[name].replace(/^[\^~]/, "")]);
  }
  return out;
}

const minor = (v) => v.split(".").slice(0, 2).join(".");

/** npm package -> Rust crate. `tauri` itself pairs with `@tauri-apps/api`. */
function rustCounterpart(npmName) {
  if (npmName === "@tauri-apps/api") return "tauri";
  // The CLI is a build tool, not a runtime pair, and versions independently.
  if (npmName === "@tauri-apps/cli") return null;
  const plugin = npmName.replace("@tauri-apps/plugin-", "");
  return `tauri-plugin-${plugin}`;
}

const cargo = readCargoLock();
const npm = readNpmVersions();
const problems = [];
let checked = 0;

for (const [npmName, npmVersions] of npm) {
  const crate = rustCounterpart(npmName);
  if (!crate) continue;
  const crateVersion = cargo.get(crate);
  // A JS-only package with no Rust side is fine; so is a Rust-only plugin
  // (tauri-plugin-fs and friends are used without their JS bindings here).
  if (!crateVersion) continue;
  checked += 1;
  for (const npmVersion of npmVersions) {
    if (minor(crateVersion) !== minor(npmVersion)) {
      problems.push(`  ${crate} (v${crateVersion})  !=  ${npmName} (v${npmVersion})`);
    }
  }
}

if (checked === 0) {
  console.error("check-tauri-pairs: matched no pairs at all — the parser is probably broken");
  process.exit(1);
}

if (problems.length > 0) {
  console.error(
    "::error::Tauri Rust crates and npm packages must share a major.minor version.\n" +
      "`tauri build` rejects these, so a release tag would fail on every platform:\n" +
      problems.join("\n") +
      "\n\nBump the npm side in package.json to match the Rust crate, then relock.",
  );
  process.exit(1);
}

console.log(`All ${checked} Tauri Rust/npm pairs agree on major.minor.`);
