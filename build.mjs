#!/usr/bin/env node
// Assembles dist/<uuid>.sdPlugin/ from assets/ plus release binaries.
//
// Usage:
//   node build.mjs                 the host target, from `cargo build --release`
//                                  (target/release/) - for local development
//   node build.mjs <triple>...     those targets, from
//                                  `cargo build --release --target <triple>`
//   node build.mjs --all           every target in manifest.json's CodePaths
//                                  (what a release ships); fails if one is missing
//
// The binary name comes from Cargo.toml and each target's file name from the
// manifest's CodePaths; both are checked against each other here, so the
// bundle can't disagree with the manifest that launches it.
import { cpSync, copyFileSync, mkdirSync, rmSync, existsSync, readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";

// The bundle directory is <UUID>.sdPlugin; every action UUID is UUID.<name>.
const UUID = "com.jfms7s.weather";

const fail = (message) => {
	console.error(message);
	process.exit(1);
};

const cargoToml = readFileSync("Cargo.toml", "utf8");
const cargoField = (field) => {
	const match = cargoToml.match(new RegExp(`^${field}\\s*=\\s*"([^"]+)"`, "m"));
	if (!match) fail(`could not find \`${field} = "..."\` in Cargo.toml`);
	return match[1];
};
const BIN_NAME = cargoField("name");
const cargoVersion = cargoField("version");

const manifest = JSON.parse(readFileSync("assets/manifest.json", "utf8"));

// Cargo.toml's [package] version and manifest.json's "Version" have nothing
// keeping them in sync - catch drift here rather than shipping a plugin
// whose crate version and Elgato-facing manifest version disagree.
if (cargoVersion !== manifest.Version) {
	fail(
		`version mismatch: Cargo.toml is ${cargoVersion} but assets/manifest.json is ${manifest.Version} - bump them together`,
	);
}

for (const action of manifest.Actions) {
	if (!action.UUID.startsWith(`${UUID}.`)) fail(`action ${action.UUID} is not under ${UUID}`);
}

const codePaths = manifest.CodePaths ?? {};
for (const [triple, file] of Object.entries(codePaths)) {
	if (file !== `${BIN_NAME}-${triple}`) {
		fail(`manifest CodePaths[${triple}] is ${file}, expected ${BIN_NAME}-${triple}`);
	}
}
for (const key of ["CodePathLin", "CodePathMac"]) {
	if (manifest[key] && !Object.values(codePaths).includes(manifest[key])) {
		fail(`manifest ${key} ${manifest[key]} is not one of its CodePaths`);
	}
}

const hostTriple = () => {
	const info = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
	const host = info.match(/^host: (\S+)$/m);
	if (!host) fail("could not read the host triple from `rustc -vV`");
	return host[1];
};

const args = process.argv.slice(2);
// [triple, path of its release binary]
let targets;
if (args.length === 0) {
	targets = [[hostTriple(), join("target", "release", BIN_NAME)]];
} else if (args.length === 1 && args[0] === "--all") {
	targets = Object.keys(codePaths).map((t) => [t, join("target", t, "release", BIN_NAME)]);
} else if (args.some((a) => a.startsWith("-"))) {
	fail("usage: node build.mjs [--all | <target-triple>...]");
} else {
	targets = args.map((t) => [t, join("target", t, "release", BIN_NAME)]);
}

for (const [triple, binPath] of targets) {
	if (!(triple in codePaths)) fail(`${triple} is not in manifest.json's CodePaths`);
	if (!existsSync(binPath)) {
		const build = binPath.startsWith(join("target", "release"))
			? "cargo build --release --locked"
			: `cargo build --release --locked --target ${triple}`;
		fail(`missing release binary: ${binPath} (run: ${build})`);
	}
}

const outDir = join("dist", `${UUID}.sdPlugin`);
rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });

cpSync("assets/manifest.json", join(outDir, "manifest.json"));
// Shipped asset directories (assets/icon-src/ holds sources and stays out).
for (const dir of ["icons", "layouts", "propertyInspector"]) {
	if (existsSync(join("assets", dir))) {
		cpSync(join("assets", dir), join(outDir, dir), { recursive: true });
	}
}
for (const [triple, binPath] of targets) {
	copyFileSync(binPath, join(outDir, codePaths[triple]));
}

const built = targets.map(([t]) => t);
const missing = Object.keys(codePaths).filter((t) => !built.includes(t));
console.log(`built ${outDir} for ${built.join(", ")}`);
if (missing.length > 0) {
	console.log(`note: no binary for ${missing.join(", ")} - fine locally; a release uses --all`);
}
