#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// Runs the native `skillpouch` binary for this computer. npm installs it as
// the optional dependency `@skillpouch/cli-<os>-<cpu>` that fits.
'use strict';
const { spawnSync } = require('node:child_process');
const path = require('node:path');

const name = `@skillpouch/cli-${process.platform}-${process.arch}`;
const exe = process.platform === 'win32' ? 'skillpouch.exe' : 'skillpouch';

let binary;
try {
	binary = path.join(path.dirname(require.resolve(`${name}/package.json`)), 'bin', exe);
} catch {
	console.error(
		`SkillPouch has no build for ${process.platform} ${process.arch} installed (${name}).\n` +
			'Reinstall without --no-optional / --omit=optional, or see https://skillpouch.net for other ways to install.',
	);
	process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: 'inherit' });
if (result.error) {
	console.error(`Couldn't start ${binary}: ${result.error.message}`);
	process.exit(1);
}
if (result.signal) {
	process.kill(process.pid, result.signal);
} else {
	process.exit(result.status ?? 1);
}
