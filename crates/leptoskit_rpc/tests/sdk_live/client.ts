// The live scenario for the generated TypeScript client; see `sdk_live.rs`.
// Runs under Deno and Node 22 or newer, next to the generated `index.ts`.

import {
	Client,
	type LiveStatus,
	type PresenceMember,
	type SdkNote,
	type SdkPage,
	type SdkPresence,
} from "./index.ts";

const env = (name: string): string => {
	const deno = (globalThis as {
		Deno?: { env: { get(name: string): string | undefined } };
	}).Deno;
	const value = deno?.env.get(name) ??
		(globalThis as { process?: { env: Record<string, string | undefined> } })
			.process?.env[name];
	if (value === undefined) {
		throw new Error(`${name} is not set`);
	}
	return value;
};

const serverUrl = env("SDK_LIVE_URL");
const prefix = env("SDK_LIVE_PREFIX");
const budget = setTimeout(() => {
	console.error("timed out");
	exit(1);
}, 60_000);

function exit(code: number): never {
	const deno = (globalThis as { Deno?: { exit(code: number): never } }).Deno;
	if (deno !== undefined) {
		deno.exit(code);
	}
	(globalThis as unknown as { process: { exit(code: number): never } }).process
		.exit(code);
	throw new Error("unreachable");
}

function check(condition: boolean, message: string): void {
	if (!condition) {
		throw new Error(message);
	}
}

/** Resolves when `test` holds, checking after every change `watch` reports. */
function until<T>(
	watch: (notify: () => void) => void,
	test: () => T | undefined,
	what: string,
): Promise<T> {
	return new Promise((resolve, reject) => {
		const timer = setTimeout(
			() => reject(new Error(`waited too long for ${what}`)),
			20_000,
		);
		const notify = () => {
			const found = test();
			if (found !== undefined) {
				clearTimeout(timer);
				resolve(found);
			}
		};
		watch(notify);
		notify();
	});
}

/** One person's client, holding the latest value of their subscription. */
function person(token: string) {
	const api = new Client({
		serverUrl,
		live: { token: () => token, minBackoffMs: 50, maxBackoffMs: 200 },
	});
	let notes: SdkNote[] | undefined;
	let page: SdkPage | undefined;
	const listeners = new Set<() => void>();
	const statuses: LiveStatus["state"][] = [];
	api.live.onStatus((status) => statuses.push(status.state));
	const stop = api.sdkNotes.subscribe({ prefix }, (value) => {
		notes = value;
		for (const listener of listeners) {
			listener();
		}
	}, (error) => console.error(`${token}: ${error.message}`));
	const stopPage = api.sdkPageRows.subscribe({ prefix }, (value) => {
		page = value;
		for (const listener of listeners) {
			listener();
		}
	});
	return {
		api,
		statuses,
		stop: () => {
			stop();
			stopPage();
		},
		notes: () => notes,
		page: () => page,
		has: (body: string) =>
			notes?.filter((note) => note.body === body).length ?? 0,
		until: <T>(test: () => T | undefined, what: string) =>
			until((notify) => listeners.add(notify), test, `${token}: ${what}`),
	};
}

const ada = person("ada");
const bob = person("bob");

try {
	await ada.until(() => ada.notes() && ada.page(), "the first values");
	await bob.until(() => bob.notes() && bob.page(), "the first values");
	check(ada.notes()!.length === 0, "a fresh database has no notes");
	console.log("ok initial");

	// Read-your-writes: when the call resolves, Bob's own subscription has it.
	const first = await bob.api.addSdkNote.live({ body: `${prefix}one` });
	check(typeof first === "string", "the mutation returns the id");
	check(
		bob.has(`${prefix}one`) === 1,
		`Bob sees his own write: ${JSON.stringify(bob.notes())}`,
	);
	check(
		bob.page()!.items.length === 1 && bob.page()!.end === first &&
			bob.page()!.has_more,
		"Bob's page rows and metadata reflect his write",
	);
	console.log("ok read-your-writes");

	// Ada's subscription receives it as a patch.
	await ada.until(
		() => ada.has(`${prefix}one`) === 1 ? true : undefined,
		"Bob's note",
	);
	console.log("ok patch");

	await bob.api.addSdkNote.live({ body: `${prefix}alpha` });
	await ada.until(
		() =>
			ada.notes()!.length === 2 && ada.page()!.items.length === 2
				? true
				: undefined,
		"an insertion before the existing row",
	);
	check(
		ada.has(`${prefix}one`) === 1 && ada.notes()![0].body === `${prefix}alpha`,
		"inserting preserves the existing row",
	);
	check(
		ada.page()!.items[0].body === `${prefix}alpha` &&
			ada.page()!.end === first && !ada.page()!.has_more,
		"page patches update rows and metadata together",
	);
	console.log("ok insertion and page");

	// The proxy lets this through, then cuts the connection before the answer.
	const dropped = await ada.api.addSdkNote.live({ body: `${prefix}drop-me` });
	check(typeof dropped === "string", "the re-sent mutation answers");
	await ada.until(
		() => ada.has(`${prefix}drop-me`) === 1 ? true : undefined,
		"the dropped note",
	);
	check(
		ada.statuses.includes("reconnecting"),
		`Ada reconnected: ${ada.statuses.join(", ")}`,
	);
	check(ada.has(`${prefix}drop-me`) === 1, "the re-sent mutation ran once");
	console.log("ok reconnect");

	const last = await bob.api.addSdkNote.live({ body: `${prefix}two` });
	await ada.until(
		() => ada.has(`${prefix}two`) === 1 ? true : undefined,
		"a note after the reconnect",
	);
	await ada.until(
		() => ada.page()!.items.length === 4 ? true : undefined,
		"the resumed page",
	);
	check(
		ada.notes()!.length === 4 && ada.page()!.end === last &&
			!ada.page()!.has_more,
		"resumed list and page preserve every row and update the end cursor",
	);
	console.log("ok resumed");

	// Channels: presence from both, and Bob's signal reaches Ada.
	const signals: { text: string; from: string | undefined }[] = [];
	let adaMembers: PresenceMember<SdkPresence>[] = [];
	let bobMembers: PresenceMember<SdkPresence>[] = [];
	const changed = new Set<() => void>();
	const poke = () => changed.forEach((listener) => listener());
	const adaRoom = ada.api.sdkRoom.join({ room: prefix }, {
		presence: { name: "Ada" },
		onPresence: (members) => {
			adaMembers = members;
			poke();
		},
		onSignal: (message, from) => {
			signals.push({ text: message.text, from });
			poke();
		},
		onError: (error) => console.error(`ada join: ${error.message}`),
	});
	const bobRoom = bob.api.sdkRoom.join({ room: prefix }, {
		presence: { name: "Bob" },
		onPresence: (members) => {
			bobMembers = members;
			poke();
		},
	});
	const names = (members: PresenceMember<SdkPresence>[]) =>
		members.map((member) => `${member.id}:${member.state.name}`).sort().join(
			",",
		);
	await until(
		(notify) => changed.add(notify),
		() =>
			names(adaMembers) === "ada:Ada,bob:Bob" &&
				names(bobMembers) === "ada:Ada,bob:Bob"
				? true
				: undefined,
		"both in the room",
	);
	bobRoom.signal({ text: "hi" });
	await until(
		(notify) => changed.add(notify),
		() => signals.length > 0 ? true : undefined,
		"Bob's signal",
	);
	check(
		signals[0].text === "hi" && signals[0].from === "bob",
		`the signal: ${JSON.stringify(signals)}`,
	);
	adaRoom.leave();
	bobRoom.leave();
	console.log("ok channel");

	ada.stop();
	bob.stop();
	ada.api.live.close();
	bob.api.live.close();
	clearTimeout(budget);
} catch (error) {
	console.error(error);
	exit(1);
}
