// The rendered comparison table: every change cell carries a direction
// marker (🟢 good way, 🟠 bad way) beside the percentage, so a reader never
// has to know which way a metric counts to read a row.
import { assertStringIncludes } from "jsr:@std/assert@1";
import { section } from "../scripts/compare.ts";
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

Deno.test("each change cell is marked with its direction", () => {
	const markdown = section(
		"Runtime (browser)",
		"base",
		"head",
		[
			// 20% slower, but the 2 ms absolute delta sits under the 5 ms warn
			// floor: ok status, still an unmistakably bad direction.
			classify(
				"table.create-1000",
				"first commit of 1,000 rows",
				"ms",
				"duration",
				10,
				12,
				policy,
			),
			// The same +20% on throughput is the good direction.
			classify(
				"clicks.throughput",
				"increment clicks per second",
				"per-sec",
				"throughput",
				1000,
				1200,
				policy,
			),
			classify("table.clear", "clear", "ms", "duration", 10, 5, policy),
			classify("table.swap", "swap rows", "ms", "duration", 10, 10, policy),
			classify(
				"table.append-1000",
				"append 1,000 rows",
				"ms",
				"duration",
				undefined,
				8,
				policy,
			),
		],
	);

	assertStringIncludes(
		markdown,
		"| metric | base | head | Δ | change | status |",
	);
	assertStringIncludes(
		markdown,
		"| first commit of 1,000 rows | 10.0 ms | 12.0 ms | 2.0 ms | 🟠 +20.0% | ➡️ ok |",
	);
	assertStringIncludes(
		markdown,
		"| increment clicks per second | 1000/s | 1200/s | 200/s | 🟢 +20.0% | ➡️ ok |",
	);
	assertStringIncludes(
		markdown,
		"| clear | 10.0 ms | 5.0 ms | 5.0 ms | 🟢 -50.0% | 🟢 improved |",
	);
	assertStringIncludes(
		markdown,
		"| swap rows | 10.0 ms | 10.0 ms | 0.000 ms | ➡️ +0.0% | ➡️ ok |",
	);
	// The base never measured it, so the head is the first run to report it:
	// new coverage, not a missing number.
	assertStringIncludes(
		markdown,
		"| append 1,000 rows | — | 8.0 ms | — | — | 🆕 new |",
	);
	// A metric only the base measured is coverage the head lost.
	assertStringIncludes(
		section(
			"Runtime (browser)",
			"base",
			"head",
			[classify(
				"table.select-row",
				"select a row",
				"ms",
				"duration",
				4,
				undefined,
				policy,
			)],
		),
		"| select a row | 4.0 ms | — | — | — | ⚪ missing |",
	);
});
