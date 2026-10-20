// Splits one `build_bench.ts` output into the two single-tool runs
// `compare.ts` expects: metrics ending in `.base` become the base run and
// metrics ending in `.head` the head run.
//
// Usage:
//   deno run -A benchmarks/scripts/split_build_results.ts \
//     --input build.json --base-out base-build.json --head-out head-build.json

import {
	type BenchmarkRun,
	type CliArguments,
	fail,
	parseArguments,
	requireString,
} from "./common.ts";

function split(
	run: BenchmarkRun,
	suffix: string,
	subject: string,
): BenchmarkRun {
	const metrics = Object.fromEntries(
		Object.entries(run.metrics)
			.filter(([id]) => id.endsWith(`.${suffix}`))
			.map((
				[id, metric],
			) => [id.replace(new RegExp(`\\.${suffix}$`), ""), metric]),
	);
	if (Object.keys(metrics).length === 0) {
		fail(`no .${suffix} metrics in ${run.subject}`);
	}
	return { ...run, subject, metrics };
}

const args: CliArguments = parseArguments(Deno.args);
const input = requireString(args, "input");
const baseOut = requireString(args, "base-out");
const headOut = requireString(args, "head-out");

const run = JSON.parse(Deno.readTextFileSync(input)) as BenchmarkRun;
await Deno.writeTextFile(
	baseOut,
	JSON.stringify(split(run, "base", "build-base"), null, "\t") + "\n",
);
await Deno.writeTextFile(
	headOut,
	JSON.stringify(split(run, "head", "build-head"), null, "\t") + "\n",
);
