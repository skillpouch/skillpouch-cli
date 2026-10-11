// SPDX-License-Identifier: AGPL-3.0-only
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';
import { PLATFORMS, build } from './npm-packages.mjs';
import {
	checkVersion,
	npmReadme,
	stampCargoLock,
	stampCargoToml,
	stampPackageJson,
} from './release.mjs';

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

test('Windows line endings are stamped too', () => {
	const toml = '[workspace.package]\r\nversion = "0.1.0"\r\nedition = "2024"\r\n';
	assert.equal(
		stampCargoToml(toml, '2.0.0'),
		'[workspace.package]\r\nversion = "2.0.0"\r\nedition = "2024"\r\n',
	);
	const lock = [
		'version = 4',
		'',
		'[[package]]',
		'name = "serde"',
		'version = "1.0.0"',
		'source = "registry+https://github.com/rust-lang/crates.io-index"',
		'',
		'[[package]]',
		'name = "skillpouch"',
		'version = "0.1.0"',
		'',
	].join('\r\n');
	const stamped = stampCargoLock(lock, '2.0.0');
	assert.match(stamped, /name = "serde"\r\nversion = "1.0.0"\r\n/);
	assert.match(stamped, /name = "skillpouch"\r\nversion = "2.0.0"\r\n/);
});

test('the npm README links to the release on GitHub', () => {
	const text = [
		'<img src=".github/assets/banner.png" alt="" />',
		'<a href="https://skillpouch.net">Website</a> · <a href="#install">Install</a>',
		'![Dashboard](.github/assets/tui-overview.webp)',
		'See [`sp-harness`](crates/sp-harness) and [the licence](./LICENSE).',
	].join('\n');
	const readme = npmReadme(text, '1.2.3');
	const raw = 'https://raw.githubusercontent.com/skillpouch/skillpouch-cli/v1.2.3';
	const blob = 'https://github.com/skillpouch/skillpouch-cli/blob/v1.2.3';
	assert.match(readme, new RegExp(`src="${raw}/.github/assets/banner.png"`));
	assert.match(readme, new RegExp(`\\(${raw}/.github/assets/tui-overview.webp\\)`));
	assert.match(readme, new RegExp(`\\(${blob}/crates/sp-harness\\)`));
	assert.match(readme, new RegExp(`\\(${blob}/LICENSE\\)`));
	assert.match(readme, /href="https:\/\/skillpouch.net"/);
	assert.match(readme, /href="#install"/);
});

test('every link in the real README works from npm', () => {
	const root = join(dirname(fileURLToPath(import.meta.url)), '..');
	const readme = npmReadme(readFileSync(join(root, 'README.md'), 'utf8'), '1.2.3');
	const links = [
		...readme.matchAll(/\b(?:src|href)="([^"]+)"/g),
		...readme.matchAll(/\]\(([^)\s]+)/g),
	].map(([, link]) => link);
	assert.ok(links.length > 10);
	for (const link of links) assert.match(link, /^(https:\/\/|#)/, link);
});
