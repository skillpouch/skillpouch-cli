// SPDX-License-Identifier: AGPL-3.0-only
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { PLATFORMS, build } from './npm-packages.mjs';
import { checkVersion, stampCargoLock, stampCargoToml, stampPackageJson } from './release.mjs';

test('only the workspace version in Cargo.toml changes', () => {
	const text = [
		'[workspace]',
		'members = ["crates/*"]',
		'',
		'[workspace.package]',
		'version = "0.1.0"',
		'edition = "2024"',
		'',
		'[workspace.dependencies]',
		'version = "9.9.9"',
	].join('\n');
	const stamped = stampCargoToml(text, '1.2.3');
	assert.match(stamped, /\[workspace.package\]\nversion = "1.2.3"/);
	assert.match(stamped, /version = "9.9.9"/);
	assert.throws(() => stampCargoToml('[package]\nversion = "1.0.0"', '1.2.3'));
});

test('Cargo.lock keeps the versions of crates from a registry', () => {
	const text = [
		'version = 4',
		'',
		'[[package]]',
		'name = "serde"',
		'version = "1.0.0"',
		'source = "registry+https://github.com/rust-lang/crates.io-index"',
		'',
		'[[package]]',
		'name = "sp-core"',
		'version = "0.1.0"',
		'dependencies = [',
		' "serde",',
		']',
		'',
		'[[package]]',
		'name = "skillpouch"',
		'version = "0.1.0"',
		'',
	].join('\n');
	const stamped = stampCargoLock(text, '2.0.0');
	assert.match(stamped, /^version = 4$/m);
	assert.match(stamped, /name = "serde"\nversion = "1.0.0"/);
	assert.match(stamped, /name = "sp-core"\nversion = "2.0.0"/);
	assert.match(stamped, /name = "skillpouch"\nversion = "2.0.0"/);
});

test('the real Cargo files can be stamped', () => {
	const root = new URL('..', import.meta.url);
	const toml = stampCargoToml(readFileSync(new URL('Cargo.toml', root), 'utf8'), '3.4.5');
	const lock = stampCargoLock(readFileSync(new URL('Cargo.lock', root), 'utf8'), '3.4.5');
	assert.match(toml, /^version = "3.4.5"$/m);
	assert.match(lock, /name = "skillpouch"\nversion = "3.4.5"/);
	assert.match(lock, /name = "sp-sync"\nversion = "3.4.5"/);
});

test('skillpouch depends on every platform package of its own version', () => {
	const stamped = JSON.parse(
		stampPackageJson('{"name":"skillpouch","version":"0.0.1","bin":{"skillpouch":"x"}}', '1.0.0'),
	);
	assert.equal(stamped.version, '1.0.0');
	assert.deepEqual(stamped.bin, { skillpouch: 'x' });
	assert.equal(Object.keys(stamped.optionalDependencies).length, Object.keys(PLATFORMS).length);
	assert.equal(stamped.optionalDependencies['@skillpouch/cli-darwin-arm64'], '1.0.0');
});

test('a release must be the version the binaries were built as', () => {
	assert.doesNotThrow(() => checkVersion('1.2.0', undefined));
	assert.doesNotThrow(() => checkVersion('1.2.0', '1.2.0'));
	assert.throws(() => checkVersion('1.3.0', '1.2.0'), /built as 1.2.0/);
	assert.throws(() => checkVersion('v1.2.0', undefined), /bad version/);
});

test('npm gets one package per platform', () => {
	const binaries = mkdtempSync(join(tmpdir(), 'binaries-'));
	const out = mkdtempSync(join(tmpdir(), 'npm-'));
	mkdirSync(binaries, { recursive: true });
	for (const target of Object.keys(PLATFORMS)) {
		const ext = target.includes('windows') ? '.exe' : '';
		writeFileSync(join(binaries, `skillpouch-${target}${ext}`), 'binary');
	}
	const platforms = build('1.2.3', binaries, out);
	assert.equal(platforms.length, Object.keys(PLATFORMS).length);
	const linux = platforms.find((p) => p.name === '@skillpouch/cli-linux-x64');
	const meta = JSON.parse(readFileSync(join(linux.dir, 'package.json'), 'utf8'));
	assert.deepEqual([meta.version, meta.os, meta.cpu], ['1.2.3', ['linux'], ['x64']]);
});
