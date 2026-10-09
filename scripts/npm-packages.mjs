#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// Builds the platform packages for one release:
//
//   node scripts/npm-packages.mjs <version> <binaries-dir> <out-dir>
//
// <binaries-dir> holds one `skillpouch-<rust target>[.exe]` per platform.
// Each becomes `@skillpouch/cli-<os>-<cpu>` (with `os` and `cpu` set, so npm
// installs only the one that fits). `skillpouch` itself (npm/skillpouch) lists
// them all as optional dependencies of the same version; its launcher runs the
// right one. semantic-release publishes it after these.
import { chmodSync, copyFileSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const PLATFORMS = {
	'x86_64-unknown-linux-musl': { os: 'linux', cpu: 'x64' },
	'aarch64-unknown-linux-musl': { os: 'linux', cpu: 'arm64' },
	'x86_64-apple-darwin': { os: 'darwin', cpu: 'x64' },
	'aarch64-apple-darwin': { os: 'darwin', cpu: 'arm64' },
	'x86_64-pc-windows-msvc': { os: 'win32', cpu: 'x64' },
	'aarch64-pc-windows-msvc': { os: 'win32', cpu: 'arm64' },
};

export const packageName = ({ os, cpu }) => `@skillpouch/cli-${os}-${cpu}`;

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

export function build(version, binaries, out) {
	if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) throw new Error(`bad version ${version}`);
	const main = JSON.parse(readFileSync(join(root, 'npm/skillpouch/package.json'), 'utf8'));
	const built = [];
	for (const file of readdirSync(binaries)) {
		const target = file.replace(/^skillpouch-/, '').replace(/\.exe$/, '');
		const platform = PLATFORMS[target];
		if (!platform) continue;
		const name = packageName(platform);
		const dir = join(out, `cli-${platform.os}-${platform.cpu}`);
		const exe = platform.os === 'win32' ? 'skillpouch.exe' : 'skillpouch';
		mkdirSync(join(dir, 'bin'), { recursive: true });
		copyFileSync(join(binaries, file), join(dir, 'bin', exe));
		chmodSync(join(dir, 'bin', exe), 0o755);
		copyFileSync(join(root, 'LICENSE'), join(dir, 'LICENSE'));
		writeFileSync(
			join(dir, 'README.md'),
			`# ${name}\n\nThe \`skillpouch\` binary for ${platform.os} ${platform.cpu}. Install \`skillpouch\` instead: it picks this package for you.\n`,
		);
		writeFileSync(
			join(dir, 'package.json'),
			`${JSON.stringify(
				{
					name,
					version,
					description: `The SkillPouch CLI for ${platform.os} ${platform.cpu}.`,
					homepage: main.homepage,
					license: main.license,
					author: main.author,
					repository: main.repository,
					os: [platform.os],
					cpu: [platform.cpu],
					files: ['bin', 'README.md', 'LICENSE'],
					preferUnplugged: true,
				},
				null,
				'\t',
			)}\n`,
		);
		built.push({ name, dir });
	}
	if (built.length === 0) throw new Error(`no binaries in ${binaries}`);
	return built;
}

/** What `skillpouch` depends on: every platform package, at the same version. */
export function optionalDependencies(version) {
	return Object.fromEntries(Object.values(PLATFORMS).map((platform) => [packageName(platform), version]));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
	const [version, binaries, out] = process.argv.slice(2);
	if (!version || !binaries || !out) {
		console.error('usage: npm-packages.mjs <version> <binaries-dir> <out-dir>');
		process.exit(2);
	}
	for (const { name, dir } of build(version, binaries, out)) console.log(`${name}\t${dir}`);
}
