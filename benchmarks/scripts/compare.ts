// Compares base and head benchmark runs and renders the PR comment.
//
// Inputs are the JSON files `runtime_bench.ts` and `build_bench.ts` produce;
// the output is one markdown document plus an exit status:
//
// - 0 — no fail-level regression (warnings are listed but do not fail);
// - 1 — a measurement problem: missing runs, missing metrics, page errors;
// - 2 — at least one fail-level regression under `benchmarks/policy.json`.
//
// Set `LEPTOSKIT_ALLOW_BENCH_REGRESSIONS=true` to record regressions in the
// comment without failing the job, for deliberate performance trade-offs.
//
// Usage:
//   deno run -A benchmarks/scripts/compare.ts \
//     --base-runtime base-runtime.json --head-runtime head-runtime.json \
//     [--base-build base-build.json --head-build head-build.json] \
//     --base-sha <sha> --head-sha <sha> \
//     --output comment.md [--status-file compare.status]

import {
	type BenchmarkRun,
	type CliArguments,
	fail,
	formatMetric,
	log,
	optionalString,
	parseArguments,
	requireString,
} from "./common.ts";
import {
	classify,
	type Comparison,
	loadPolicy,
	statusEmoji,
} from "./policy.ts";

function readRunSync(path: string): BenchmarkRun {
	try {
		return JSON.parse(Deno.readTextFileSync(path)) as BenchmarkRun;
	} catch (error) {
		fail(`could not read benchmark run ${path}: ${(error as Error).message}`);
	}
}

function compareRuns(
	base: BenchmarkRun,
	head: BenchmarkRun,
	policy: Awaited<ReturnType<typeof loadPolicy>>,
): Comparison[] {
	const ids = new Set([
		...Object.keys(base.metrics),
		...Object.keys(head.metrics),
	]);
	const comparisons: Comparison[] = [];
	for (const id of ids) {
		const baseMetric = base.metrics[id];
		const headMetric = head.metrics[id];
		comparisons.push(
			classify(
				id,
				headMetric?.label ?? baseMetric?.label ?? id,
				(baseMetric ?? headMetric).unit,
				(baseMetric ?? headMetric).class,
				baseMetric?.median,
				headMetric?.median,
				policy,
			),
		);
	}
	return comparisons;
}

function section(
	title: string,
	baseSubject: string,
	headSubject: string,
	comparisons: Comparison[],
): string {
	const lines = [
		`### ${title}`,
		"",
		`| metric | ${baseSubject} | ${headSubject} | Δ | change | status |`,
		"| --- | ---: | ---: | ---: | ---: | --- |",
	];
	for (const comparison of comparisons) {
		const base = comparison.base === null
			? "—"
			: formatMetric(comparison.base, comparison.unit);
		const head = comparison.head === null
			? "—"
			: formatMetric(comparison.head, comparison.unit);
		const delta = comparison.delta === null
			? "—"
			: formatMetric(Math.abs(comparison.delta), comparison.unit);
		const percent = comparison.percent === null
			? "—"
			: `${comparison.percent >= 0 ? "+" : ""}${
				comparison.percent.toFixed(1)
			}%`;
		lines.push(
			`| ${comparison.label} | ${base} | ${head} | ${delta} | ${percent} | ${
				statusEmoji(comparison.status)
			} ${comparison.status} |`,
		);
	}
	return lines.join("\n");
}

async function main(): Promise<void> {
	const args: CliArguments = parseArguments(Deno.args);
	const baseRuntimePath = requireString(args, "base-runtime");
	const headRuntimePath = requireString(args, "head-runtime");
	const baseBuildPath = optionalString(args, "base-build");
	const headBuildPath = optionalString(args, "head-build");
	const baseSha = optionalString(args, "base-sha") ?? "base";
	const headSha = optionalString(args, "head-sha") ?? "head";
	const output = optionalString(args, "output");
	const statusFile = optionalString(args, "status-file");

	const policy = await loadPolicy(
		new URL("../policy.json", import.meta.url).pathname,
	);
	const baseRuntime = readRunSync(baseRuntimePath);
	const headRuntime = readRunSync(headRuntimePath);

	const baseShaShort = baseSha.slice(0, 7);
	const headShaShort = headSha.slice(0, 7);

	const missing = [];
	if (Object.keys(headRuntime.metrics).length === 0) {
		missing.push("head runtime");
	}
	if (Object.keys(baseRuntime.metrics).length === 0) {
		missing.push("base runtime");
	}

	const sections: string[] = [
		"## 📊 leptoskit benchmarks",
		"",
		`base \`${baseShaShort}\` → head \`${headShaShort}\` · runtime: headless Chromium, median of ${headRuntime.runs} runs · thresholds: \`benchmarks/policy.json\``,
		"",
	];

	const runtimeComparisons = compareRuns(baseRuntime, headRuntime, policy);
	sections.push(
		section("Runtime (browser)", "base", "head", runtimeComparisons),
		"",
	);

	let buildComparisons: Comparison[] = [];
	if (baseBuildPath && headBuildPath) {
		const baseBuild = readRunSync(baseBuildPath);
		const headBuild = readRunSync(headBuildPath);
		buildComparisons = compareRuns(baseBuild, headBuild, policy);
		sections.push(
			section("Build speed (fixture app)", "base", "head", buildComparisons),
			"",
		);
	}

	const all = [...runtimeComparisons, ...buildComparisons];
	const fails = all.filter((comparison) => comparison.status === "fail");
	const warns = all.filter((comparison) => comparison.status === "warn");
	const missingMetrics = all.filter((comparison) =>
		comparison.status === "missing"
	);

	if (missing.length > 0 || missingMetrics.length > 0) {
		sections.push(
			`> ⚠️ incomplete comparison: ${
				[...missing, ...missingMetrics.map((metric) => metric.id)].join(", ")
			}`,
			"",
		);
	}
	if (fails.length > 0) {
		sections.push(
			`> 🔴 **${fails.length} regression${
				fails.length === 1 ? "" : "s"
			} over the fail threshold:** ${
				fails.map((metric) => `\`${metric.id}\``).join(", ")
			}`,
			"",
		);
	} else if (warns.length > 0) {
		sections.push(
			`> 🟡 ${warns.length} warning${warns.length === 1 ? "" : "s"}: ${
				warns.map((metric) => `\`${metric.id}\``).join(", ")
			}`,
			"",
		);
	} else {
		sections.push("> 🟢 no performance regressions", "");
	}

	const markdown = sections.join("\n");
	if (output) {
		await Deno.writeTextFile(output, markdown);
		log(`wrote ${output}`);
	} else {
		console.log(markdown);
	}
	if (statusFile) {
		await Deno.writeTextFile(statusFile, "");
	}

	const allowRegressions =
		Deno.env.get("LEPTOSKIT_ALLOW_BENCH_REGRESSIONS") === "true";
	if (missing.length > 0) {
		Deno.exit(1);
	}
	if (fails.length > 0 && !allowRegressions) {
		Deno.exit(2);
	}
}

await main();
