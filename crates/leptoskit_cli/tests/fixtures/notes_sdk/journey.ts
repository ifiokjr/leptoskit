// A fresh `--db sqlite` scaffold's published API, from the TypeScript client
// `leptoskit rpc generate` writes for it; see `db.rs`. Runs under Node 22 or
// newer, next to the generated `index.ts`, with nothing installed.
//
// 1. A one-shot call reads `health`, and the live `notes` subscription gets
//    the notes the harness added over HTTP.
// 2. `addNote.live` resolves once the subscription shows the new note.
// 3. The harness stops the server, which tells the socket to reconnect, and
//    starts it again. The client reconnects, subscribes again, and receives
//    the note the harness adds afterwards.

import { Client, type LiveStatus, type PublicNote } from "./index.ts";

const env = (name: string): string => {
	const value = (globalThis as {
		process?: { env: Record<string, string | undefined> };
	}).process?.env[name];
	if (value === undefined) {
		throw new Error(`${name} is not set`);
	}
	return value;
};

const exit = (code: number): never =>
	(globalThis as unknown as { process: { exit(code: number): never } })
		.process.exit(code);

const budget = setTimeout(() => {
	console.error("timed out");
	exit(1);
}, 120_000);

function check(condition: boolean, message: string): void {
	if (!condition) {
		throw new Error(message);
	}
}

const api = new Client({
	serverUrl: env("NOTES_URL"),
	live: { minBackoffMs: 100, maxBackoffMs: 500 },
});
const statuses: LiveStatus["state"][] = [];
let notes: PublicNote[] | undefined;
const listeners = new Set<() => void>();
const changed = () => listeners.forEach((listener) => listener());
api.live.onStatus((status) => {
	statuses.push(status.state);
	changed();
});
const stop = api.notes.subscribe((value) => {
	notes = value;
	changed();
}, (error) => console.error(`notes: ${error.message}`));

/** Resolves when `test` holds, checking after every change. */
function until(test: () => boolean, what: string): Promise<void> {
	return new Promise((resolve, reject) => {
		const timer = setTimeout(
			() =>
				reject(
					new Error(
						`waited too long for ${what}: ${JSON.stringify(notes)} ${
							statuses.join(", ")
						}`,
					),
				),
			60_000,
		);
		const listener = () => {
			if (test()) {
				clearTimeout(timer);
				listeners.delete(listener);
				resolve();
			}
		};
		listeners.add(listener);
		listener();
	});
}

const has = (body: string) =>
	notes?.filter((note) => note.body === body).length ?? 0;

try {
	const health = await api.health();
	check(health.ready === true, `health: ${JSON.stringify(health)}`);
	await until(() => has("hello") === 1, "the note added over HTTP");
	console.log("ok subscribed");

	// Read-your-writes: when the call resolves, the subscription has it.
	const added = await api.addNote.live({ body: "from the sdk" });
	check(added.body === "from the sdk", `the mutation: ${JSON.stringify(added)}`);
	check(has("from the sdk") === 1, `the write shows: ${JSON.stringify(notes)}`);
	check(notes![0].id === added.id, "the newest note comes first");
	console.log("ok mutation");

	// The one-shot form of the same mutation, then of the query.
	await api.addNote({ body: "one shot" });
	const listed = await api.notes();
	check(
		listed.some((note) => note.body === "one shot"),
		`the one-shot list: ${JSON.stringify(listed)}`,
	);
	await until(() => has("one shot") === 1, "the one-shot note, live");
	console.log("ready for restart");

	await until(
		() => has("after restart") === 1,
		"the note added after the restart",
	);
	check(
		statuses.includes("reconnecting"),
		`the client reconnected: ${statuses.join(", ")}`,
	);
	check(
		has("from the sdk") === 1 && has("one shot") === 1,
		`the resumed list keeps every note: ${JSON.stringify(notes)}`,
	);
	console.log("ok reconnect");

	stop();
	api.live.close();
	clearTimeout(budget);
} catch (error) {
	console.error(error);
	exit(1);
}
