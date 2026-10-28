import { deepStrictEqual, equal, throws } from "node:assert/strict";
import {
	BUSY,
	NATIVE,
	NativeCall,
	NativePlugin,
	NativePlugins,
	PluginError,
	UNAVAILABLE,
} from "./plugins.ts";

type Completed = [number, number, number[]];

/** Completions in arrival order, with reply bytes as plain arrays. */
function dispatcher(): { plugins: NativePlugins; completed: Completed[] } {
	const completed: Completed[] = [];
	const plugins = new NativePlugins((id, code, payload) => {
		completed.push([id, code, Array.from(new Uint8Array(payload))]);
	});
	return { plugins, completed };
}

/** A plugin whose calls the test settles by hand. */
class Manual implements NativePlugin {
	readonly name: string;
	readonly methods = [0, 1];
	calls: NativeCall[] = [];
	settle: ((reply: ArrayBuffer | Error) => void)[] = [];
	cancelled = 0;
	closed = 0;
	closeError: Error | undefined = undefined;

	constructor(name = "manual") {
		this.name = name;
	}

	invoke(call: NativeCall): Promise<ArrayBuffer> {
		this.calls.push(call);
		call.onCancel(() => this.cancelled++);
		return new Promise((resolve, reject) => {
			this.settle.push((reply) =>
				reply instanceof Error ? reject(reply) : resolve(reply)
			);
		});
	}

	close(): void {
		this.closed++;
		if (this.closeError !== undefined) {
			throw this.closeError;
		}
	}
}

const bytes = (...values: number[]) => new Uint8Array(values).buffer;
const turn = () => new Promise((resolve) => setTimeout(resolve, 0));

Deno.test("calls reach their plugin and replies arrive on a later turn", async () => {
	const { plugins, completed } = dispatcher();
	const manual = new Manual();
	plugins.add(7, manual);
	plugins.start(1, 7, 1, bytes(9, 8));
	deepStrictEqual(Array.from(new Uint8Array(manual.calls[0].payload)), [9, 8]);
	equal(manual.calls[0].method, 1);
	manual.settle[0](bytes(4, 2));
	deepStrictEqual(
		completed,
		[],
		"a settled promise never re-enters the runtime synchronously",
	);
	await turn();
	deepStrictEqual(completed, [[1, 0, [4, 2]]]);
});

Deno.test("unknown plugins and methods, plugin errors, and thrown failures", async () => {
	const { plugins, completed } = dispatcher();
	const manual = new Manual();
	plugins.add(0, manual);
	plugins.start(1, 5, 0, bytes());
	plugins.start(2, 0, 9, bytes());
	plugins.start(3, 0, 0, bytes());
	plugins.start(4, 0, 0, bytes());
	manual.settle[0](new PluginError(BUSY));
	const report = console.error;
	const logged: string[] = [];
	console.error = (message: string) => logged.push(message);
	try {
		manual.settle[1](new Error("display service crashed"));
		await turn();
	} finally {
		console.error = report;
	}
	deepStrictEqual(completed, [
		[1, UNAVAILABLE, []],
		[2, UNAVAILABLE, []],
		[3, BUSY, []],
		[4, NATIVE, []],
	]);
	equal(logged.length, 1);
	throws(() => plugins.add(0, new Manual()), /Duplicate/);
});

Deno.test("guest cancellation notifies the plugin and discards its reply", async () => {
	const { plugins, completed } = dispatcher();
	const manual = new Manual();
	plugins.add(0, manual);
	plugins.start(1, 0, 1, bytes());
	plugins.cancel(1);
	plugins.cancel(1);
	plugins.cancel(99);
	equal(manual.cancelled, 1);
	equal(manual.calls[0].cancelled, true);
	manual.settle[0](bytes(1));
	await turn();
	deepStrictEqual(completed, []);
	// A handler added after cancellation runs at once; a settled call is
	// never cancelled.
	let late = 0;
	manual.calls[0].onCancel(() => late++);
	equal(late, 1);
	plugins.start(2, 0, 0, bytes());
	manual.settle[1](bytes());
	await turn();
	plugins.cancel(2);
	equal(manual.cancelled, 1);
	deepStrictEqual(completed, [[2, 0, []]]);
});

Deno.test("close cancels outstanding calls, closes every plugin, and admits nothing", async () => {
	const { plugins, completed } = dispatcher();
	const failing = new Manual("failing");
	failing.closeError = new Error("first close failure");
	const healthy = new Manual("healthy");
	plugins.add(0, failing);
	plugins.add(1, healthy);
	plugins.start(1, 0, 0, bytes());
	plugins.start(2, 1, 0, bytes());
	throws(() => plugins.close(), /first close failure/);
	deepStrictEqual([failing.cancelled, healthy.cancelled], [1, 1]);
	deepStrictEqual([failing.closed, healthy.closed], [1, 1]);
	plugins.close();
	equal(healthy.closed, 1);
	plugins.start(3, 1, 0, bytes());
	healthy.settle[0](bytes());
	await turn();
	equal(healthy.calls.length, 1);
	deepStrictEqual(completed, []);
});
