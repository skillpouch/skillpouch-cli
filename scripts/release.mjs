#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// The steps semantic-release runs (see .releaserc.json), also used by the
// build jobs:
//
//   node scripts/release.mjs verify <version>   check the version, note it in .release-version
//   node scripts/release.mjs stamp <version>    write it into Cargo.toml, Cargo.lock and npm/skillpouch
//   node scripts/release.mjs publish <version>  publish the platform packages built from $BINARIES
//
// The version is planned once (a dry run), the binaries are built with it, and
// the real run refuses to release anything else: `RELEASE_VERSION` holds the
// planned one.
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, optionalDependencies } from './npm-packages.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const VERSION = /^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/;

export function checkVersion(version, planned) {
	if (!VERSION.test(version ?? '')) throw new Error(`bad version ${version}`);
	if (planned && planned !== version) {
		throw new Error(
			`the binaries were built as ${planned}, but the commits now make ${version}; the next run releases it`,
		);
	}
}

/** Sets `version` in the `[workspace.package]` table. */
export function stampCargoToml(text, version) {
	let inTable = false;
	let stamped = false;
	const lines = text.split('\n').map((line) => {
		if (/^\s*\[/.test(line)) inTable = line.trim() === '[workspace.package]';
		if (inTable && !stamped && /^version\s*=/.test(line)) {
			stamped = true;
			return `version = "${version}"`;
		}
		return line;
	});
	if (!stamped) throw new Error('Cargo.toml has no [workspace.package] version');
	return lines.join('\n');
}

/** Sets the version of every workspace crate: the packages without a `source`. */
export function stampCargoLock(text, version) {
	return text
		.split('\n[[package]]\n')
		.map((block, index) =>
			index === 0 || /^source = /m.test(block)
				? block
				: block.replace(/^version = ".*"$/m, `version = "${version}"`),
		)
		.join('\n[[package]]\n');
}

/** `skillpouch` on npm: its version and the platform packages it runs. */
export function stampPackageJson(text, version) {
	const stamped = {
		...JSON.parse(text),
		version,
		optionalDependencies: optionalDependencies(version),
	};
	return `${JSON.stringify(stamped, null, '\t')}\n`;
}

function rewrite(path, change) {
	const file = join(root, path);
	writeFileSync(file, change(readFileSync(file, 'utf8')));
}

function stamp(version) {
	rewrite('Cargo.toml', (text) => stampCargoToml(text, version));
	rewrite('Cargo.lock', (text) => stampCargoLock(text, version));
	rewrite('npm/skillpouch/package.json', (text) => stampPackageJson(text, version));
}

function published(name, version) {
	try {
		execFileSync('npm', ['view', `${name}@${version}`, 'version'], { stdio: 'pipe' });
		return true;
	} catch {
		return false;
	}
}

// Runs before @semantic-release/npm publishes `skillpouch`, which depends on these.
function publish(version) {
	const binaries = process.env.BINARIES ?? join(root, 'binaries');
	const out = mkdtempSync(join(tmpdir(), 'skillpouch-npm-'));
	for (const { name, dir } of build(version, binaries, out)) {
		if (published(name, version)) {
			console.error(`${name}@${version} is already on npm`);
			continue;
		}
		execFileSync('npm', ['publish', '--access', 'public'], { cwd: dir, stdio: 'inherit' });
	}
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
	const [step, version] = process.argv.slice(2);
	checkVersion(version, process.env.RELEASE_VERSION);
	if (step === 'verify') writeFileSync(join(root, '.release-version'), `${version}\n`);
	else if (step === 'stamp') stamp(version);
	else if (step === 'publish') publish(version);
	else {
		console.error('usage: release.mjs verify|stamp|publish <version>');
		process.exit(2);
	}
}
