#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WEB_DIR="$ROOT_DIR/web_ui/timem-web"

if [ ! -d "$WEB_DIR/node_modules/.pnpm" ]; then
  echo "error: web dependencies are not installed; run pnpm --dir web_ui/timem-web install --frozen-lockfile" >&2
  exit 1
fi

node <<'NODE'
const fs = require("fs");
const path = require("path");

const storeRoot = path.join("web_ui", "timem-web", "node_modules", ".pnpm");
const allowed = new Set([
  "0BSD",
  "Apache-2.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "CC-BY-4.0",
  "ISC",
  "MIT",
  "MIT AND BSD-3-Clause",
]);
const disallowedPattern = /\b(AGPL|GPL|LGPL)\b/i;
const packages = new Map();

function walk(directory) {
  let entries = [];
  try {
    entries = fs.readdirSync(directory, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    const fullPath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      walk(fullPath);
    } else if (entry.isFile() && entry.name === "package.json") {
      inspectPackageJson(fullPath);
    }
  }
}

function inspectPackageJson(packageJsonPath) {
  let pkg;
  try {
    pkg = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));
  } catch {
    return;
  }
  if (typeof pkg.name !== "string" || !pkg.name) return;
  if (!isPnpmPackageRoot(packageJsonPath, pkg.name)) return;
  const key = `${pkg.name}@${pkg.version || ""}`;
  if (packages.has(key)) return;
  const license = normalizeLicense(pkg.license || pkg.licenses);
  packages.set(key, { license, path: packageJsonPath });
}

function isPnpmPackageRoot(packageJsonPath, packageName) {
  const normalized = packageJsonPath.split(path.sep).join("/");
  const suffix = packageName.startsWith("@")
    ? `/node_modules/${packageName}/package.json`
    : `/node_modules/${packageName}/package.json`;
  return normalized.includes("/.pnpm/") && normalized.endsWith(suffix);
}

function normalizeLicense(value) {
  if (typeof value === "string") return value.trim();
  if (Array.isArray(value)) {
    return value
      .map((item) => typeof item === "string" ? item : item && item.type)
      .filter(Boolean)
      .join(" OR ");
  }
  if (value && typeof value.type === "string") return value.type.trim();
  return "";
}

walk(storeRoot);

const failures = [];
for (const [key, metadata] of packages) {
  const license = metadata.license;
  if (!license) {
    failures.push(`${key}: missing license (${metadata.path})`);
  } else if (disallowedPattern.test(license)) {
    failures.push(`${key}: disallowed copyleft license ${license} (${metadata.path})`);
  } else if (!allowed.has(license)) {
    failures.push(`${key}: review unclassified license ${license} (${metadata.path})`);
  }
}

if (failures.length > 0) {
  console.error("web dependency license check failed:");
  for (const failure of failures) console.error(`- ${failure}`);
  process.exit(1);
}

const counts = new Map();
for (const { license } of packages.values()) {
  counts.set(license, (counts.get(license) || 0) + 1);
}
const summary = [...counts.entries()]
  .sort((left, right) => left[0].localeCompare(right[0]))
  .map(([license, count]) => `${license}:${count}`)
  .join(" ");
console.log(`web_license_check: ok (${packages.size} packages; ${summary})`);
NODE
