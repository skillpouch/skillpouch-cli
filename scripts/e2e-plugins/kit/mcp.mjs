// A stdio MCP server with one tool. It notes in /tmp/marks/mcp.log each
// start, handshake and request for its tools, and whether it found the
// plugin's folder.
import { appendFileSync, existsSync, mkdirSync } from 'node:fs';
import { createInterface } from 'node:readline';

const root = process.env.CLAUDE_PLUGIN_ROOT ?? '';
const found = existsSync(`${root}/.claude-plugin/plugin.json`);
const mark = (what) => {
	mkdirSync('/tmp/marks', { recursive: true });
	appendFileSync('/tmp/marks/mcp.log', `${what} ${found ? 'found' : `missing:${root}`}\n`);
};
mark('start');

const reply = (id, result) =>
	process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', id, result })}\n`);
for await (const line of createInterface({ input: process.stdin })) {
	let message;
	try {
		message = JSON.parse(line);
	} catch {
		continue;
	}
	if (message.id === undefined) continue;
	if (message.method === 'initialize') {
		mark('init');
		reply(message.id, {
			protocolVersion: message.params?.protocolVersion ?? '2025-06-18',
			capabilities: { tools: {} },
			serverInfo: { name: 'kit-mcp', version: '1.0.0' },
		});
	} else if (message.method === 'tools/list') {
		mark('tools');
		reply(message.id, {
			tools: [
				{
					name: 'kit_ping',
					description: 'Answers pong.',
					inputSchema: { type: 'object', properties: {} },
				},
			],
		});
	} else if (message.method === 'tools/call') {
		reply(message.id, { content: [{ type: 'text', text: 'pong' }] });
	} else {
		reply(message.id, {});
	}
}
