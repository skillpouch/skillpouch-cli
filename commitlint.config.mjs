// SPDX-License-Identifier: AGPL-3.0-only
// Commit messages are Conventional Commits: semantic-release reads them to
// decide the next version and write the changelog. See AGENTS.md.
export default {
	extends: ['@commitlint/config-conventional'],
	ignores: [(message) => message.startsWith('chore(release):')],
	rules: {
		'header-max-length': [2, 'always', 100],
		'body-max-line-length': [0],
		'footer-max-line-length': [0],
		'scope-enum': [
			2,
			'always',
			[
				'cli',
				'tui',
				'sync',
				'harness',
				'api',
				'core',
				'crypto',
				'usage',
				'npm',
				'release',
				'ci',
				'deps',
			],
		],
	},
};
