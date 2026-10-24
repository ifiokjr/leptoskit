import { assertEquals } from "jsr:@std/assert@1";
import { classify, type Policy } from "../scripts/policy.ts";

const policy: Policy = {
	duration: {
		warn: { percent: 10, absolute: 5 },
		fail: { percent: 25, absolute: 15 },
	},
	startup: {
		warn: { percent: 15, absolute: 10 },
		fail: { percent: 30, absolute: 25 },
	},
	bytes: {
		warn: { percent: 2, absolute: 1024 },
		fail: { percent: 10, absolute: 5120 },
	},
	memory: {
		warn: { percent: 20, absolute: 2_097_152 },
		fail: { percent: 50, absolute: 5_242_880 },
	},
	throughput: {
		warn: { percent: 8, absolute: 500 },
		fail: { percent: 20, absolute: 1500 },
	},
	build: {
		warn: { percent: 10, absolute: 100 },
		fail: { percent: 30, absolute: 500 },
	},
};

Deno.test("a regression needs both the percent and the absolute threshold", () => {
	// 20% slower but only 3 ms absolute: under the 5 ms warn floor.
	assertEquals(
		classify("t", "t", "ms", "duration", 15, 18, policy).status,
		"ok",
	);
	// 20% slower and 6 ms absolute: over warn (10%/5ms), under fail.
	assertEquals(
		classify("t", "t", "ms", "duration", 30, 36, policy).status,
		"warn",
	);
	// 30% slower and 30 ms absolute: over fail (25%/15ms).
	assertEquals(
		classify("t", "t", "ms", "duration", 100, 130, policy).status,
		"fail",
	);
});

Deno.test("direction says which way head moved, whatever the sign", () => {
	// Durations count down: an increase is worse, a decrease better.
	assertEquals(
		classify("t", "t", "ms", "duration", 5.4, 6.1, policy).direction,
		"worse",
	);
	assertEquals(
		classify("t", "t", "ms", "duration", 100, 80, policy).direction,
		"better",
	);
	// Throughput counts up: the same signs flip their meaning.
	assertEquals(
		classify("t", "t", "per-sec", "throughput", 1000, 1200, policy).direction,
		"better",
	);
	assertEquals(
		classify("t", "t", "per-sec", "throughput", 1200, 1000, policy).direction,
		"worse",
	);
	// No movement is not a direction either way.
	assertEquals(
		classify("t", "t", "ms", "duration", 42, 42, policy).direction,
		"unchanged",
	);
});

Deno.test("higher-is-better metrics invert the comparison", () => {
	// Throughput dropped 25% (2,000 → 1,500): over fail (20%/1500? no —
	// absolute delta is 500, under 1500, over 8%/500 warn).
	assertEquals(
		classify("t", "t", "per-sec", "throughput", 2000, 1500, policy).status,
		"warn",
	);
	// A large absolute drop crosses fail.
	assertEquals(
		classify("t", "t", "per-sec", "throughput", 10000, 6000, policy).status,
		"fail",
	);
	// An increase is fine.
	assertEquals(
		classify("t", "t", "per-sec", "throughput", 1000, 2000, policy).status,
		"improved",
	);
});

Deno.test("improvements beyond the warn threshold are marked improved", () => {
	assertEquals(
		classify("t", "t", "ms", "duration", 100, 80, policy).status,
		"improved",
	);
	assertEquals(
		classify("t", "t", "ms", "duration", 100, 97, policy).status,
		"ok",
	);
});

Deno.test("missing base or head is reported, not guessed", () => {
	assertEquals(
		classify("t", "t", "ms", "duration", undefined, 10, policy).status,
		"missing",
	);
	assertEquals(
		classify("t", "t", "ms", "duration", 10, undefined, policy).status,
		"missing",
	);
	// With a side missing there is no direction to mark either.
	assertEquals(
		classify("t", "t", "ms", "duration", undefined, 10, policy).direction,
		null,
	);
});

Deno.test("transfer bytes are deterministic, so their floors are tight", () => {
	// Identical sources differ by a few bytes of gzip framing at most.
	assertEquals(
		classify("t", "t", "bytes", "bytes", 64_000, 64_040, policy).status,
		"ok",
	);
	// 5 KB of extra wasm on a 64 KB payload (+8%) is a warning worth reading.
	assertEquals(
		classify("t", "t", "bytes", "bytes", 64_000, 69_000, policy).status,
		"warn",
	);
	// 10% and 5 KB cross both fail thresholds.
	assertEquals(
		classify("t", "t", "bytes", "bytes", 64_000, 71_000, policy).status,
		"fail",
	);
});

Deno.test("heap readings wobble, so memory floors are generous", () => {
	// Identical builds have been observed ±1.1 MB apart on a 6 MB heap.
	assertEquals(
		classify("t", "t", "bytes", "memory", 5_760_000, 6_900_000, policy).status,
		"ok",
	);
	// Doubling the heap is a regression whatever the noise.
	assertEquals(
		classify("t", "t", "bytes", "memory", 5_760_000, 12_000_000, policy).status,
		"fail",
	);
});
