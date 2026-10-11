#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// A stand-in model provider for scripts/e2e-settings.sh. It answers every
// request with an error, so the assistant stops at once, and writes one JSON
// line per request: what the assistant made of its synced settings (the base
// URL it called, the key it sent, the model it asked for).
//
//   node mock-llm.mjs <port> <log-file>
import { appendFileSync } from 'node:fs';
import { createServer } from 'node:http';

const [port, log] = process.argv.slice(2);

createServer((request, response) => {
	let body = '';
	request.on('data', (chunk) => (body += chunk));
	request.on('end', () => {
		let model = null;
		try {
			model = JSON.parse(body).model ?? null;
		} catch {}
		// Gemini puts the model in the path: /v1beta/models/<model>:generateContent
		model ??= request.url.match(/\/models\/([^/:?]+)/)?.[1] ?? null;
		const url = new URL(request.url, 'http://mock');
		const key =
			request.headers['x-api-key'] ??
			request.headers['x-goog-api-key'] ??
			url.searchParams.get('key') ??
			request.headers.authorization?.replace(/^Bearer\s+/i, '') ??
			null;
		appendFileSync(
			log,
			`${JSON.stringify({ method: request.method, path: url.pathname, key, model })}\n`,
		);
		response.writeHead(400, { 'content-type': 'application/json' });
		response.end(
			JSON.stringify({
				error: { type: 'invalid_request_error', code: 400, message: 'mock provider' },
			}),
		);
	});
}).listen(Number(port), '127.0.0.1');
