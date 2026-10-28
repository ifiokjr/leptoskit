import { useEffect, useState } from "react";
import { StyleSheet, Text, View } from "react-native";
import type { Spec } from "../../specs/NativeArenaEcho";
import {
	failureReport,
	median,
	nextFrame,
	percentile,
	type Phase,
	round3,
	TEXT_COLOR,
	writeReport,
} from "../arena";
import { echoModule } from "../host";
import { Screen } from "../Screen";

const WARM_UP_CALLS = 200;
const TIMED_CALLS = 2000;
const BATCH_SIZE = 100;

const STRING_1K = Array.from(
	{ length: 1024 },
	(_, index) => String.fromCharCode(97 + (index % 26)),
).join("");

const BYTES_64K = (() => {
	const bytes = new Uint8Array(65536);
	for (let index = 0; index < bytes.length; index += 1) {
		bytes[index] = index % 251;
	}
	return bytes;
})();
const WORDS_64K = new Uint32Array(BYTES_64K.buffer);

const OBJECT_100: Record<string, number> = Object.fromEntries(
	Array.from({ length: 100 }, (_, index) => [`f${index}`, index]),
);

type Tier = "async" | "sync";
type Payload = "none" | "int" | "str1k" | "bytes64k" | "obj";

interface Probe<T> {
	payload: Payload;
	call: () => T;
	verify: (result: Awaited<T>) => boolean;
}

function sameBytes(buffer: ArrayBuffer): boolean {
	if (buffer.byteLength !== BYTES_64K.byteLength) {
		return false;
	}
	const words = new Uint32Array(buffer);
	for (let index = 0; index < words.length; index += 1) {
		if (words[index] !== WORDS_64K[index]) {
			return false;
		}
	}
	return true;
}

function sameObject(value: object): boolean {
	const entries: Array<[string, unknown]> = Object.entries(value);
	return entries.length === 100 &&
		entries.every(([key, field]) => OBJECT_100[key] === field);
}

/** Encodes bytes for the string bridge, chunked so `fromCharCode` never sees 64 Ki arguments. */
function toBase64(bytes: Uint8Array): string {
	let binary = "";
	for (let offset = 0; offset < bytes.length; offset += 8192) {
		binary += String.fromCharCode(...bytes.subarray(offset, offset + 8192));
	}
	return btoa(binary);
}

function fromBase64(text: string): ArrayBuffer {
	const binary = atob(text);
	const bytes = new Uint8Array(binary.length);
	for (let index = 0; index < binary.length; index += 1) {
		bytes[index] = binary.charCodeAt(index);
	}
	return bytes.buffer;
}

function asyncProbes(echo: Spec): Probe<Promise<unknown>>[] {
	return [
		{
			payload: "none",
			call: () => echo.echo(),
			verify: (result) => result === null || result === undefined,
		},
		{
			payload: "int",
			call: () => echo.addOne(41),
			verify: (result) => result === 42,
		},
		{
			payload: "str1k",
			call: () => echo.echoString(STRING_1K),
			verify: (result) => result === STRING_1K,
		},
		{
			payload: "bytes64k",
			// The timed call includes encoding and decoding: that is what moving bytes over this tier costs.
			call: async () => fromBase64(await echo.echoBase64(toBase64(BYTES_64K))),
			verify: (result) => result instanceof ArrayBuffer && sameBytes(result),
		},
		{
			payload: "obj",
			call: () => echo.echoObject(OBJECT_100),
			verify: (result) =>
				typeof result === "object" && result !== null && sameObject(result),
		},
	];
}

function syncProbes(echo: Spec): Probe<unknown>[] {
	return [
		{
			payload: "none",
			call: () => echo.echoSync(),
			verify: (result) => result === null || result === undefined,
		},
		{
			payload: "int",
			call: () => echo.addOneSync(41),
			verify: (result) => result === 42,
		},
		{
			payload: "str1k",
			call: () => echo.echoStringSync(STRING_1K),
			verify: (result) => result === STRING_1K,
		},
		{
			payload: "bytes64k",
			call: () => echo.echoBytesSync(BYTES_64K.buffer),
			verify: (result) => result instanceof ArrayBuffer && sameBytes(result),
		},
		{
			payload: "obj",
			call: () => echo.echoObjectSync(OBJECT_100),
			verify: (result) =>
				typeof result === "object" && result !== null && sameObject(result),
		},
	];
}

interface Timing {
	durationsUs: number[];
	totalMs: number;
}

async function timeAsync(probe: Probe<Promise<unknown>>): Promise<Timing> {
	for (let index = 0; index < WARM_UP_CALLS; index += 1) {
		if (!probe.verify(await probe.call())) {
			throw new Error(`async ${probe.payload}: wrong result during warm-up`);
		}
	}
	const durationsUs: number[] = [];
	let totalMs = 0;
	for (let index = 0; index < TIMED_CALLS; index += 1) {
		const start = performance.now();
		const result = await probe.call();
		const elapsed = performance.now() - start;
		totalMs += elapsed;
		durationsUs.push(elapsed * 1000);
		if (!probe.verify(result)) {
			throw new Error(`async ${probe.payload}: wrong result on call ${index}`);
		}
	}
	return { durationsUs, totalMs };
}

function timeSync(probe: Probe<unknown>): Timing {
	for (let index = 0; index < WARM_UP_CALLS; index += 1) {
		if (!probe.verify(probe.call())) {
			throw new Error(`sync ${probe.payload}: wrong result during warm-up`);
		}
	}
	const durationsUs: number[] = [];
	let totalMs = 0;
	for (let index = 0; index < TIMED_CALLS; index += 1) {
		const start = performance.now();
		const result = probe.call();
		const elapsed = performance.now() - start;
		totalMs += elapsed;
		durationsUs.push(elapsed * 1000);
		if (!probe.verify(result)) {
			throw new Error(`sync ${probe.payload}: wrong result on call ${index}`);
		}
	}
	return { durationsUs, totalMs };
}

/**
 * The smallest step `performance.now()` takes, so a reader can tell whether
 * per-call latencies below it mean anything.
 */
function clockResolutionMs(): number {
	let smallest = Number.POSITIVE_INFINITY;
	let previous = performance.now();
	for (let index = 0; index < 10000; index += 1) {
		const now = performance.now();
		if (now > previous) {
			smallest = Math.min(smallest, now - previous);
		}
		previous = now;
	}
	return smallest;
}

/** p50, p99, and calls per second from all 2,000 calls; samples hold the mean of each batch of 100. */
function summarise(
	tier: Tier,
	payload: Payload,
	{ durationsUs, totalMs }: Timing,
	metrics: Record<string, number>,
	samples: Record<string, number[]>,
): void {
	const key = `${tier}_${payload}`;
	metrics[`${key}_p50_us`] = round3(median(durationsUs));
	metrics[`${key}_p99_us`] = round3(percentile(durationsUs, 99));
	metrics[`${key}_per_s`] = Math.round((TIMED_CALLS / totalMs) * 1000);
	const batches: number[] = [];
	for (let offset = 0; offset < durationsUs.length; offset += BATCH_SIZE) {
		const batch = durationsUs.slice(offset, offset + BATCH_SIZE);
		batches.push(
			round3(batch.reduce((sum, value) => sum + value, 0) / batch.length),
		);
	}
	samples[`${key}_p50_us`] = batches;
}

/**
 * Round trips to the ArenaEcho TurboModule: the async tier awaits Promise
 * methods (idiomatic), the sync tier calls synchronous methods (tuned).
 * Both tiers go in one report.
 */
export function Echo() {
	const [phase, setPhase] = useState<Phase>("idle");
	const [progress, setProgress] = useState("");

	useEffect(() => {
		async function run(): Promise<void> {
			if (echoModule === null) {
				throw new Error("no native layer on the web");
			}
			setPhase("running");
			await nextFrame();
			const metrics: Record<string, number> = {};
			const samples: Record<string, number[]> = {};
			metrics.clock_resolution_us = round3(clockResolutionMs() * 1000);
			for (const probe of asyncProbes(echoModule)) {
				setProgress(`async ${probe.payload}`);
				await nextFrame();
				summarise(
					"async",
					probe.payload,
					await timeAsync(probe),
					metrics,
					samples,
				);
			}
			for (const probe of syncProbes(echoModule)) {
				setProgress(`sync ${probe.payload}`);
				await nextFrame();
				summarise("sync", probe.payload, timeSync(probe), metrics, samples);
			}
			setProgress("");
			setPhase("done");
			writeReport({
				scenario: "echo",
				lane: "idiomatic",
				ok: true,
				notes: [
					"async tier: Promise TurboModule methods; sync tier: synchronous TurboModule methods",
					"async bytes64k is passed as base64 both ways (Promise<ArrayBuffer> is unsupported for Objective-C and Kotlin TurboModules), and its timing includes encoding and decoding; sync bytes64k passes an ArrayBuffer",
					"sync none returns null: a void TurboModule method is queued without a round trip",
				],
				metrics,
				samples,
			});
		}

		run().catch((error: unknown) => {
			setPhase("failed");
			writeReport(failureReport("echo", error));
		});
	}, []);

	return (
		<Screen name="echo" phase={phase}>
			<View style={styles.body}>
				<Text style={styles.progress}>{progress}</Text>
			</View>
		</Screen>
	);
}

const styles = StyleSheet.create({
	body: {
		padding: 16,
	},
	progress: {
		color: TEXT_COLOR,
		fontSize: 16,
	},
});
