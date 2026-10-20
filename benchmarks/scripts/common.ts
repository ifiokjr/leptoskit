// Shared types, paths, and helpers for the benchmark scripts. No external
// imports: the scripts run offline in CI from the repository alone.

/** `benchmarks/` — the parent of this scripts directory. */
export const BENCHMARKS_ROOT = new URL("..", import.meta.url).pathname.replace(
	/\/$/,
	"",
);
export const REPO_ROOT = BENCHMARKS_ROOT.replace(/\/benchmarks$/, "");
export const RUNTIME_APPS_ROOT = `${BENCHMARKS_ROOT}/apps/runtime`;
export const SHARED_BENCH_JS = `${RUNTIME_APPS_ROOT}/shared/bench.js`;

export type MetricUnit = "ms" | "bytes" | "per-sec" | "count";

/** The policy class a metric belongs to; see `policy.json`. */
export type MetricClass =
	| "duration"
	| "startup"
	| "bytes"
	| "memory"
	| "throughput"
	| "build";

export interface Metric {
	unit: MetricUnit;
	class: MetricClass;
	label: string;
	values: number[];
	median: number;
	min: number;
	max: number;
}

export interface BenchmarkRun {
	schema: 1;
	/** What produced this run: an app name, a tool name, "base", "head". */
	subject: string;
	timestamp: string;
	gitCommit: string | null;
	environment: {
		browser: string;
		platform: string;
		cpus: number | null;
	};
	runs: number;
	/** Why a layer was skipped, when a producer had to leave one out. */
	notes?: string[];
	metrics: Record<string, Metric>;
}

export function emptyMetric(
	unit: MetricUnit,
	metricClass: MetricClass,
	label: string,
): { unit: MetricUnit; class: MetricClass; label: string; values: number[] } {
	return { unit, class: metricClass, label, values: [] };
}

export function aggregate(run: {
	subject: string;
	environment: BenchmarkRun["environment"];
	runs: number;
	metrics: Record<
		string,
		{ unit: MetricUnit; class: MetricClass; label: string; values: number[] }
	>;
}): BenchmarkRun {
	const metrics: Record<string, Metric> = {};
	for (const [id, metric] of Object.entries(run.metrics)) {
		if (metric.values.length === 0) continue;
		const sorted = [...metric.values].sort((left, right) => left - right);
		const middle = Math.floor(sorted.length / 2);
		const median = sorted.length % 2 === 1
			? sorted[middle]
			: (sorted[middle - 1] + sorted[middle]) / 2;
		metrics[id] = {
			...metric,
			median,
			min: sorted[0],
			max: sorted[sorted.length - 1],
		};
	}
	return {
		schema: 1,
		subject: run.subject,
		timestamp: new Date().toISOString(),
		gitCommit: currentCommit(),
		environment: run.environment,
		runs: run.runs,
		metrics,
	};
}

export function currentCommit(): string | null {
	try {
		const output = new Deno.Command("git", {
			args: ["rev-parse", "HEAD"],
			cwd: REPO_ROOT,
		}).outputSync();
		return output.success
			? new TextDecoder().decode(output.stdout).trim()
			: null;
	} catch {
		return null;
	}
}

/** Copies the shared `bench.js` into every runtime app that serves it. */
export async function syncBenchGlue(appDirs: string[]): Promise<void> {
	const glue = await Deno.readTextFile(SHARED_BENCH_JS);
	for (const dir of appDirs) {
		await Deno.writeTextFile(`${dir}/bench.js`, glue);
	}
}

export function formatMetric(value: number, unit: MetricUnit): string {
	switch (unit) {
		case "ms":
			return value >= 100
				? `${value.toFixed(0)} ms`
				: value >= 1
				? `${value.toFixed(1)} ms`
				: `${value.toFixed(3)} ms`;
		case "bytes":
			return value >= 1024 * 1024
				? `${(value / 1024 / 1024).toFixed(2)} MB`
				: value >= 1024
				? `${(value / 1024).toFixed(1)} KB`
				: `${value.toFixed(0)} B`;
		case "per-sec":
			return `${value.toFixed(0)}/s`;
		case "count":
			return value.toFixed(0);
	}
}

export function log(message: string): void {
	console.log(`\x1b[2m[bench] ${message}\x1b[0m`);
}

export function fail(message: string): never {
	console.error(`\x1b[31m[bench] ${message}\x1b[0m`);
	Deno.exit(1);
}

export interface CliArguments {
	flags: Map<string, string | boolean | string[]>;
	positional: string[];
}

export function parseArguments(argv: string[]): CliArguments {
	const flags = new Map<string, string | boolean | string[]>();
	const positional: string[] = [];
	for (let index = 0; index < argv.length; index += 1) {
		const argument = argv[index];
		if (argument.startsWith("--")) {
			const [name, inline] = argument.slice(2).split("=", 2);
			const value = inline !== undefined
				? inline
				: argv[index + 1] && !argv[index + 1].startsWith("--")
				? argv[index + 1]
				: true;
			if (inline === undefined && value !== true) index += 1;
			const existing = flags.get(name);
			if (existing === undefined) {
				flags.set(name, value);
			} else if (Array.isArray(existing)) {
				existing.push(String(value));
			} else {
				flags.set(name, [String(existing), String(value)]);
			}
		} else {
			positional.push(argument);
		}
	}
	return { flags, positional };
}

export function requireString(arguments_: CliArguments, name: string): string {
	const value = arguments_.flags.get(name);
	if (typeof value !== "string" || value.length === 0) {
		fail(`missing required flag --${name}`);
	}
	return value;
}

export function optionalString(
	arguments_: CliArguments,
	name: string,
): string | undefined {
	const value = arguments_.flags.get(name);
	return typeof value === "string" ? value : undefined;
}

export function optionalNumber(
	arguments_: CliArguments,
	name: string,
): number | undefined {
	const value = arguments_.flags.get(name);
	if (value === undefined) return undefined;
	const parsed = Number(value);
	return Number.isFinite(parsed) ? parsed : undefined;
}
