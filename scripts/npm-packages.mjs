#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// Builds the npm packages for one release:
//
//   node scripts/npm-packages.mjs <version> <binaries-dir> <out-dir>
//
// <binaries-dir> holds one `skillpouch-<rust target>[.exe]` per platform.
// Each becomes `@skillpouch/cli-<os>-<cpu>` (with `os` and `cpu` set, so npm
// installs only the one that fits), and `skillpouch` itself lists them all as
// optional dependencies of the same version; its launcher runs the right one.
import { chmodSync, copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
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

	const launcher = join(out, 'skillpouch');
	mkdirSync(join(launcher, 'bin'), { recursive: true });
	for (const file of ['bin/skillpouch.js', 'README.md', 'LICENSE']) {
		const from = existsSync(join(root, 'npm/skillpouch', file))
			? join(root, 'npm/skillpouch', file)
			: join(root, file);
		copyFileSync(from, join(launcher, file));
	}
	const optionalDependencies = Object.fromEntries(
		Object.values(PLATFORMS).map((platform) => [packageName(platform), version]),
	);
	writeFileSync(
		join(launcher, 'package.json'),
		`${JSON.stringify({ ...main, version, optionalDependencies }, null, '\t')}\n`,
	);
	return { platforms: built, launcher };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
	const [version, binaries, out] = process.argv.slice(2);
	if (!version || !binaries || !out) {
		console.error('usage: npm-packages.mjs <version> <binaries-dir> <out-dir>');
		process.exit(2);
	}
	const { platforms, launcher } = build(version, binaries, out);
	for (const { name, dir } of platforms) console.log(`${name}\t${dir}`);
	console.log(`skillpouch\t${launcher}`);
}
