#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// The browser half of scripts/e2e-settings.sh: one signed-in Chromium page in
// the web app, driven one command per line on stdin, answering one line on
// stdout. It stays open while a computer logs in, because the browser seals
// the account keys to the new device.
//
//   account               a new account; answers `<email> <pouch id>`
//   token                 a one-time `skillpouch login` token
//   connected <name>      waits until the computer <name> is connected
//   items                 the Settings items the pouch lists, comma separated
//   share <name> <s1,s2>  shares the Settings item <name>, opens the link in a
//                         fresh browser; answers `ok <text length>` when none
//                         of the secrets show, else `leak <secret>`
//   quit
//
// Needs the web app's dependencies: SKILLPOUCH_WEB_DIR (apps/web of the
// skillpouch repository) and its Playwright Chromium.
import { randomUUID } from 'node:crypto';
import { createRequire } from 'node:module';
import { join } from 'node:path';
import { createInterface } from 'node:readline';

const webDir = process.env.SKILLPOUCH_WEB_DIR;
const origin = process.env.SKILLPOUCH_WEB_ORIGIN;
if (!webDir || !origin) throw new Error('set SKILLPOUCH_WEB_DIR and SKILLPOUCH_WEB_ORIGIN');
const { chromium } = createRequire(join(webDir, 'package.json'))('@playwright/test');

const PASSWORD = 'correct horse battery staple 42';
const browser = await chromium.launch(
	process.env.PLAYWRIGHT_CHROMIUM_PATH
		? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_PATH }
		: {},
);
const page = await (await browser.newContext({ baseURL: origin })).newPage();
page.setDefaultTimeout(30_000);
let pouchId = '';

async function account() {
	const email = `settings-${randomUUID().slice(0, 8)}@skillpouch.test`;
	await page.goto('/sign-in');
	const form = page.getByRole('form', { name: 'Development sign-in' });
	await form.getByLabel('Email').fill(email);
	await form.getByRole('button', { name: 'Sign in' }).click();
	await page.getByLabel('Master password').fill(PASSWORD);
	await page.getByLabel('Confirm password').fill(PASSWORD);
	await page.getByRole('button', { name: 'Continue' }).click();
	await page.getByRole('checkbox', { name: 'I saved my Recovery Key' }).click();
	await page.getByRole('button', { name: 'Continue' }).click();
	await page.getByRole('button', { name: 'Finish setup' }).click();
	await page.waitForURL(/\/app\/?$/, { timeout: 60_000 });
	await page.getByRole('link', { name: /Personal/ }).first().click();
	await page.waitForURL(/\/app\/p\/[0-9a-f-]+\/?$/);
	pouchId = page.url().match(/\/app\/p\/([0-9a-f-]+)/)[1];
	return `${email} ${pouchId}`;
}

async function token() {
	await page.goto('/app/account/devices');
	await page.getByRole('button', { name: 'Connect CLI' }).first().click();
	const command = page.getByTestId('connect-command');
	await command.waitFor();
	return (await command.getAttribute('data-command')).match(/sp_lt_[A-Za-z0-9_-]{32}/)[0];
}

async function connected(name) {
	await page.getByTestId('connected-device').filter({ hasText: name }).waitFor({ timeout: 120_000 });
	await page.keyboard.press('Escape');
	return 'ok';
}

async function openSettings() {
	await page.goto(`/app/p/${pouchId}`);
	await page.getByRole('navigation').getByRole('link', { name: /^Settings/ }).first().click();
	await page.getByRole('heading', { name: 'Settings' }).first().waitFor();
}

async function items() {
	await openSettings();
	await page.getByRole('row').nth(1).waitFor();
	const names = await page
		.getByRole('button', { name: /^Actions for / })
		.evaluateAll((buttons) =>
			buttons.map((b) => b.getAttribute('aria-label').replace(/^Actions for /, '')),
		);
	return names.sort().join(',');
}

async function share(name, secrets) {
	await openSettings();
	await page.getByRole('button', { name: `Actions for ${name}` }).click();
	await page.getByRole('menuitem', { name: 'Share' }).click();
	const dialog = page.getByRole('dialog', { name: `Share ${name}` });
	await dialog.getByRole('button', { name: 'Create link' }).click();
	const url = await dialog.getByLabel('Link').inputValue();
	await dialog.getByRole('button', { name: 'Done' }).click();
	const stranger = await (await browser.newContext()).newPage();
	await stranger.goto(url);
	await stranger.getByRole('heading', { name }).waitFor();
	await stranger.getByText('"harness"').first().waitFor();
	const text = await stranger.locator('body').innerText();
	await stranger.context().close();
	const leak = secrets.split(',').find((secret) => secret && text.includes(secret));
	return leak ? `leak ${leak}` : `ok ${text.length}`;
}

for await (const line of createInterface({ input: process.stdin })) {
	const [command, ...args] = line.trim().split(/\s+/);
	try {
		const answer =
			command === 'account'
				? await account()
				: command === 'token'
					? await token()
					: command === 'connected'
						? await connected(args[0])
						: command === 'items'
							? await items()
							: command === 'share'
								? await share(args[0], args[1] ?? '')
								: command === 'quit'
									? null
									: `error unknown command ${command}`;
		if (answer === null) break;
		process.stdout.write(`${answer}\n`);
	} catch (error) {
		process.stdout.write(`error ${String(error.message ?? error).split('\n')[0]}\n`);
	}
}
await browser.close();
