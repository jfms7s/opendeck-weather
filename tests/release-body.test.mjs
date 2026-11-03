// Runs scripts/release-body.mjs and checks the draft notes it prints.
// No dependencies: `node --test tests/*.test.mjs`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const body = execFileSync(process.execPath, ["scripts/release-body.mjs"], { cwd: root, encoding: "utf8" });

test("the verify line names the attestation check before the checksum, as the README does", () => {
	const attest = body.indexOf(
		"gh attestation verify opendeck-weather.streamDeckPlugin --repo jfms7s/opendeck-weather --signer-workflow jfms7s/opendeck-weather/.github/workflows/release.yml",
	);
	const checksum = body.indexOf("sha256sum -c SHA256SUMS");
	assert.ok(attest >= 0, "attestation command with --signer-workflow missing");
	assert.ok(checksum > attest, "checksum must come after the attestation check");
	assert.match(body, /damaged download/);
});

test("the notes still start the smoke-test section unverified", () => {
	assert.match(body, /^UNVERIFIED: /m);
	assert.match(body, /^- \[ \] /m);
});
