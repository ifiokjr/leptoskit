import { useEffect, useState } from "@lynx-js/react";

import type { ScenarioProps } from "../App.js";
import {
	CLOCK_IS_NATIVE_CALL,
	CLOCK_SOURCE,
	HAS_HIGH_RESOLUTION_CLOCK,
	median,
	nextFrame,
	now,
	percentile,
	readLaunch,
	round3,
} from "../arena.js";
import type { ArenaEchoModule, EchoObject } from "../typing.js";

const WARMUP_CALLS = 200;
const TIMED_CALLS = 2000;
const BATCH = 100;

type Tier = "async" | "sync";
type PayloadName = "none" | "int" | "str1k" | "bytes64k" | "obj";

/** One payload: how to call it through each tier, and how to check what came back. */
interface Payload<Value, Result> {
	name: PayloadName;
	value: Value;
	callAsync(module: ArenaEchoModule, value: Value): Promise<Result>;
	callSync(module: ArenaEchoModule, value: Value): Result;
	verify(result: Result): boolean;
}

const STRING_1K = Array.from(
	{ length: 1024 },
	(_, index) => String.fromCharCode(33 + (index % 94)),
).join("");

function bytes64k(): ArrayBuffer {
	const bytes = new Uint8Array(65_536);
	for (let index = 0; index < bytes.length; index += 1) {
		bytes[index] = index % 251;
	}
	return bytes.buffer;
}

const BYTES_64K = bytes64k();
const BYTES_64K_WORDS = new Uint32Array(BYTES_64K);

function sameBytes(result: ArrayBuffer): boolean {
	if (
		!(result instanceof ArrayBuffer) ||
		result.byteLength !== BYTES_64K.byteLength
	) return false;
	const words = new Uint32Array(result);
	for (let index = 0; index < words.length; index += 1) {
		if (words[index] !== BYTES_64K_WORDS[index]) return false;
	}
	return true;
}

const OBJECT_100: EchoObject = Object.fromEntries(
	Array.from({ length: 100 }, (_, index) => [`f${index}`, index]),
);

function sameObject(result: EchoObject): boolean {
	if (typeof result !== "object" || result === null) return false;
	for (let index = 0; index < 100; index += 1) {
		if (result[`f${index}`] !== index) return false;
	}
	return Object.keys(result).length === 100;
}

function callback<Result>(
	invoke: (resolve: (result: Result) => void) => void,
): Promise<Result> {
	return new Promise((resolve) => invoke(resolve));
}

const NONE: Payload<undefined, undefined> = {
	name: "none",
	value: undefined,
	callAsync: (module) =>
		callback<undefined>((resolve) => module.echo(() => resolve(undefined))),
	callSync: (module) => {
		module.echoSync();
		return undefined;
	},
	verify: () => true,
};

const INT: Payload<number, number> = {
	name: "int",
	value: 41,
	callAsync: (module, value) =>
		callback((resolve) => module.addOne(value, resolve)),
	callSync: (module, value) => module.addOneSync(value),
	verify: (result) => result === 42,
};

const STR1K: Payload<string, string> = {
	name: "str1k",
	value: STRING_1K,
	callAsync: (module, value) =>
		callback((resolve) => module.echoString(value, resolve)),
	callSync: (module, value) => module.echoStringSync(value),
	verify: (result) => result === STRING_1K,
};

const BYTES: Payload<ArrayBuffer, ArrayBuffer> = {
	name: "bytes64k",
	value: BYTES_64K,
	callAsync: (module, value) =>
		callback((resolve) => module.echoBytes(value, resolve)),
	callSync: (module, value) => module.echoBytesSync(value),
	verify: sameBytes,
};

const OBJ: Payload<EchoObject, EchoObject> = {
	name: "obj",
	value: OBJECT_100,
	callAsync: (module, value) =>
		callback((resolve) => module.echoObject(value, resolve)),
	callSync: (module, value) => module.echoObjectSync(value),
	verify: sameObject,
};

interface Measured {
	durationsUs: number[];
	totalMs: number;
}

/**
 * The cost of one clock read when the clock is a native module call (iOS):
 * the median gap between back-to-back reads. Each timed call spans one
 * read's worth of overhead, which the ladder subtracts.
 */
function clockOverheadMs(): number {
	if (!CLOCK_IS_NATIVE_CALL) return 0;
	const gaps: number[] = [];
	for (let sample = 0; sample < 1000; sample += 1) {
		const start = now();
		gaps.push(now() - start);
	}
	return median(gaps);
}

async function measure<Value, Result>(
	module: ArenaEchoModule,
	tier: Tier,
	payload: Payload<Value, Result>,
	overheadMs: number,
): Promise<Measured> {
	const label = `${tier} ${payload.name}`;
	const once = async (): Promise<number> => {
		let result: Result;
		let elapsed: number;
		if (tier === "async") {
			const start = now();
			result = await payload.callAsync(module, payload.value);
			elapsed = now() - start;
		} else {
			const start = now();
			result = payload.callSync(module, payload.value);
			elapsed = now() - start;
		}
		if (!payload.verify(result)) throw new Error(`${label}: wrong result`);
		return Math.max(0, elapsed - overheadMs);
	};
	for (let call = 0; call < WARMUP_CALLS; call += 1) await once();
	const durationsUs: number[] = [];
	let totalMs = 0;
	for (let call = 0; call < TIMED_CALLS; call += 1) {
		const elapsed = await once();
		durationsUs.push(elapsed * 1000);
		totalMs += elapsed;
	}
	return { durationsUs, totalMs };
}

/**
 * Echo ladder: five payloads, each 200 warm-up and 2,000 timed sequential
 * round trips through the `ArenaEcho` native module, for both tiers. Lynx
 * native modules are reached only from the background thread. A method with
 * a callback is the async tier: the native side runs on the background
 * thread and the callback comes back as a new task. A method that returns a
 * value is the sync tier: the call returns it directly.
 */
export function Echo(props: ScenarioProps) {
	const { setPhase, finish } = props;
	const [progress, setProgress] = useState("waiting");

	useEffect(() => {
		async function run(): Promise<void> {
			const module = NativeModules.ArenaEcho;
			if (module === undefined || readLaunch().platform === "web") {
				throw new Error("no native layer on the web");
			}
			await nextFrame();
			setPhase("running");
			const metrics: Record<string, number> = {};
			const samples: Record<string, number[]> = {};
			const overheadMs = clockOverheadMs();
			const payloads: {
				name: PayloadName;
				measure(tier: Tier): Promise<Measured>;
			}[] = [
				{
					name: NONE.name,
					measure: (tier) => measure(module, tier, NONE, overheadMs),
				},
				{
					name: INT.name,
					measure: (tier) => measure(module, tier, INT, overheadMs),
				},
				{
					name: STR1K.name,
					measure: (tier) => measure(module, tier, STR1K, overheadMs),
				},
				{
					name: BYTES.name,
					measure: (tier) => measure(module, tier, BYTES, overheadMs),
				},
				{
					name: OBJ.name,
					measure: (tier) => measure(module, tier, OBJ, overheadMs),
				},
			];
			for (const tier of ["async", "sync"] as const) {
				for (const payload of payloads) {
					setProgress(`${tier} ${payload.name}`);
					const { durationsUs, totalMs } = await payload.measure(tier);
					const sorted = [...durationsUs].sort((a, b) => a - b);
					const key = `${tier}_${payload.name}`;
					metrics[`${key}_p50_us`] = round3(percentile(sorted, 0.5));
					metrics[`${key}_p99_us`] = round3(percentile(sorted, 0.99));
					metrics[`${key}_per_s`] = round3(TIMED_CALLS / (totalMs / 1000));
					const batches: number[] = [];
					for (let start = 0; start < durationsUs.length; start += BATCH) {
						const batch = durationsUs.slice(start, start + BATCH);
						batches.push(
							round3(
								batch.reduce((sum, value) => sum + value, 0) / batch.length,
							),
						);
					}
					samples[`${key}_p50_us`] = batches;
				}
			}
			const notes = [
				"64 KiB bytes cross as an ArrayBuffer (NSData on iOS, byte[] on Android)",
				"async tier: a native module method with a callback; sync tier: a method returning its result, called on the background thread",
				"samples hold the mean of each consecutive batch of 100 timed calls",
			];
			notes.push(`timed with ${CLOCK_SOURCE}`);
			if (CLOCK_IS_NATIVE_CALL) {
				notes.push(
					`each timing subtracts one clock read (${
						round3(overheadMs * 1000)
					} us, the median back-to-back gap)`,
				);
			}
			if (!HAS_HIGH_RESOLUTION_CLOCK) {
				notes.push("Date.now has 1 ms resolution");
			}
			setProgress("finished");
			finish({ ok: true, metrics, samples, notes });
		}

		run().catch((error: unknown) => {
			setProgress("failed");
			finish({
				ok: false,
				error: error instanceof Error ? error.message : String(error),
				metrics: {},
			});
		});
	}, []);

	return (
		<view className="fill">
			<text className="row">{progress}</text>
		</view>
	);
}
