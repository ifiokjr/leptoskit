// Regenerates the benchmark tables in `docs/src/benchmarks.md` from the
// committed external results. Only the regions between the generated
// markers change; the prose around them is hand-maintained.
//
// Usage:
//   deno run -A benchmarks/scripts/docs_update.ts \
//     --runtime benchmarks/results/external-runtime.json \
//     --build benchmarks/results/external-build.json [--check]

import {
	type BenchmarkRun,
	type CliArguments,
	formatMetric,
	log,
	optionalString,
	parseArguments,
	requireString,
} from "./common.ts";

const DOCS_PATH =
	new URL("../../docs/src/benchmarks.md", import.meta.url).pathname;
const RUNTIME_BEGIN = "<!-- BEGIN GENERATED: benchmarks-runtime -->";
const RUNTIME_END = "<!-- END GENERATED: benchmarks-runtime -->";
const BUILD_BEGIN = "<!-- BEGIN GENERATED: benchmarks-build -->";
const BUILD_END = "<!-- END GENERATED: benchmarks-build -->";
const NATIVE_BEGIN = "<!-- BEGIN GENERATED: benchmarks-native -->";
const NATIVE_END = "<!-- END GENERATED: benchmarks-native -->";
const RESULTS_DIR = new URL("../results/", import.meta.url).pathname;

function readRunSync(path: string): BenchmarkRun {
	return JSON.parse(Deno.readTextFileSync(path)) as BenchmarkRun;
}

/** `leptoskit.table.create-1000` → app `leptoskit`, metric `table.create-1000`. */
function splitMetricId(id: string): { app: string; metric: string } | null {
	const separator = id.indexOf(".");
	if (separator === -1) return null;
	return { app: id.slice(0, separator), metric: id.slice(separator + 1) };
}

/** One line of provenance so a reader knows what machine produced a table. */
function provenance(run: BenchmarkRun, includeBrowser: boolean): string {
	const date = run.timestamp.slice(0, 10);
	const cores = run.environment.cpus === null
		? ""
		: `, ${run.environment.cpus} cores`;
	const browser = includeBrowser
		? (run.environment.browser.match(/Chrome\/\d+/)?.[0] ??
			run.environment.browser)
		: "";
	const browserNote = browser ? `, headless ${browser}` : "";
	const sampling = includeBrowser
		? `${run.runs} runs per app, medians`
		: "hyperfine wall-clock medians, tools interleaved per round";
	return `_Measured ${date} on ${run.environment.platform}${cores}${browserNote}; ${sampling}._`;
}

function renderRuntimeTable(run: BenchmarkRun): string {
	const apps = new Map<string, Map<string, { median: number; unit: string }>>();
	for (const [id, metric] of Object.entries(run.metrics)) {
		const parts = splitMetricId(id);
		if (!parts) continue;
		if (!apps.has(parts.app)) apps.set(parts.app, new Map());
		apps.get(parts.app)!.set(parts.metric, {
			median: metric.median,
			unit: metric.unit,
		});
	}

	const metricColumns: { id: string; label: string }[] = [
		{ id: "startup.mounted", label: "first render" },
		{ id: "startup.transfer", label: "transfer" },
		{ id: "table.create-1000", label: "create 1k" },
		{ id: "table.create-10000", label: "create 10k" },
		{ id: "table.update-every-10th", label: "update 10th" },
		{ id: "table.swap-rows", label: "swap" },
		{ id: "table.remove-row", label: "remove" },
		{ id: "table.select-row", label: "select" },
		{ id: "table.append-1000", label: "append 1k" },
		{ id: "table.clear", label: "clear" },
		{ id: "clicks.throughput", label: "clicks/s" },
		{ id: "memory.heap-after-create-10000", label: "heap" },
	];

	const header = ["app", ...metricColumns.map((column) => column.label)];
	const lines = [
		`| ${header.join(" | ")} |`,
		`| ${header.map(() => "---:").join(" | ")} |`,
	];
	for (const [app, metrics] of apps) {
		const cells = [app];
		for (const column of metricColumns) {
			const metric = metrics.get(column.id);
			cells.push(
				metric ? formatMetric(metric.median, metric.unit as "ms") : "—",
			);
		}
		lines.push(`| ${cells.join(" | ")} |`);
	}
	lines.push("", provenance(run, true));
	return lines.join("\n");
}

function renderBuildTable(run: BenchmarkRun): string {
	const tools = new Map<string, Map<string, { median: number }>>();
	for (const [id, metric] of Object.entries(run.metrics)) {
		// build.<scenario>.<tool>
		const parts = id.split(".");
		if (parts.length < 3 || parts[0] !== "build") continue;
		const scenario = parts[1];
		const tool = parts.slice(2).join(".");
		if (!tools.has(tool)) tools.set(tool, new Map());
		tools.get(tool)!.set(scenario, { median: metric.median });
	}

	const lines = [
		"| tool | warm rebuild | incremental rebuild | cold build |",
		"| --- | ---: | ---: | ---: |",
	];
	for (const [tool, scenarios] of tools) {
		const cell = (scenario: string) =>
			scenarios.has(scenario)
				? `${scenarios.get(scenario)!.median.toFixed(0)} ms`
				: "—";
		lines.push(
			`| ${tool} | ${cell("warm")} | ${cell("incremental")} | ${
				cell("cold")
			} |`,
		);
	}
	lines.push("", provenance(run, false));
	return lines.join("\n");
}

/**
 * The native, wasm, and JavaScript layers: one column per platform result
 * file (`results/native-<platform>.json`), one row per metric, in layer order.
 */
function renderNativeTable(
	runs: { platform: string; run: BenchmarkRun }[],
): string {
	const order = ["native.", "yoga.", "wasm.", "js."];
	const ids = new Set<string>();
	for (const { run } of runs) {
		for (const id of Object.keys(run.metrics)) ids.add(id);
	}
	const sorted = [...ids].sort((left, right) => {
		const rank = (id: string) =>
			order.findIndex((prefix) => id.startsWith(prefix));
		return rank(left) - rank(right) || left.localeCompare(right);
	});
	const header = ["metric", ...runs.map(({ platform }) => platform)];
	const lines = [
		`| ${header.join(" | ")} |`,
		`| --- | ${runs.map(() => "---:").join(" | ")} |`,
	];
	for (const id of sorted) {
		const label = runs.map(({ run }) => run.metrics[id]?.label).find(Boolean) ??
			id;
		const cells = runs.map(({ run }) => {
			const metric = run.metrics[id];
			return metric ? formatMetric(metric.median, metric.unit) : "—";
		});
		lines.push(`| ${label} | ${cells.join(" | ")} |`);
	}
	const first = runs[0]?.run;
	if (first) {
		const where = runs
			.map(({ platform, run }) =>
				`${platform}: ${run.environment.platform}${
					run.environment.cpus === null ? "" : `, ${run.environment.cpus} cores`
				}`
			)
			.join("; ");
		lines.push(
			"",
			`_Measured ${
				first.timestamp.slice(0, 10)
			} — ${where}; ${first.runs} runs per scenario, medians._`,
		);
	}
	return lines.join("\n");
}

/** Every `results/native-<platform>.json`, in a stable platform order. */
function nativeRuns(): { platform: string; run: BenchmarkRun }[] {
	const found: { platform: string; run: BenchmarkRun }[] = [];
	try {
		for (const entry of Deno.readDirSync(RESULTS_DIR)) {
			const match = entry.name.match(/^native-([a-z0-9]+)\.json$/);
			if (match) {
				found.push({
					platform: match[1],
					run: readRunSync(`${RESULTS_DIR}${entry.name}`),
				});
			}
		}
	} catch {
		// no results directory yet
	}
	const rank = (platform: string) =>
		["host", "ios", "android"].indexOf(platform);
	return found.sort((left, right) =>
		rank(left.platform) - rank(right.platform)
	);
}

function splice(
	content: string,
	begin: string,
	end: string,
	replacement: string,
): string {
	const startIndex = content.indexOf(begin);
	const endIndex = content.indexOf(end);
	if (startIndex === -1 || endIndex === -1 || endIndex < startIndex) {
		throw new Error(`docs markers missing or out of order: ${begin} … ${end}`);
	}
	return content.slice(0, startIndex + begin.length) + "\n" + replacement +
		"\n" +
		content.slice(endIndex);
}

/**
 * Runs the document through dprint, the repository's formatter, so generated
 * tables land in exactly the shape `dprint fmt` produces (it pads markdown
 * table columns). Without dprint on PATH the unformatted table is returned.
 */
async function formatWithDprint(content: string): Promise<string> {
	const which = new Deno.Command("which", { args: ["dprint"] }).outputSync();
	if (!which.success) return content;
	const temp = await Deno.makeTempFile({ suffix: ".md" });
	try {
		await Deno.writeTextFile(temp, content);
		const formatted = new Deno.Command("dprint", { args: ["fmt", temp] })
			.outputSync();
		if (!formatted.success) return content;
		return await Deno.readTextFile(temp);
	} finally {
		await Deno.remove(temp);
	}
}

async function main(): Promise<void> {
	const args: CliArguments = parseArguments(Deno.args);
	const runtimePath = requireString(args, "runtime");
	const buildPath = optionalString(args, "build");
	const check = args.flags.get("check") === true;

	const runtimeRun = readRunSync(runtimePath);
	let updated = splice(
		Deno.readTextFileSync(DOCS_PATH),
		RUNTIME_BEGIN,
		RUNTIME_END,
		renderRuntimeTable(runtimeRun),
	);
	if (buildPath) {
		const buildRun = readRunSync(buildPath);
		updated = splice(
			updated,
			BUILD_BEGIN,
			BUILD_END,
			renderBuildTable(buildRun),
		);
	}
	const native = nativeRuns();
	if (native.length > 0 && updated.includes(NATIVE_BEGIN)) {
		updated = splice(
			updated,
			NATIVE_BEGIN,
			NATIVE_END,
			renderNativeTable(native),
		);
	}
	updated = await formatWithDprint(updated);

	const current = Deno.readTextFileSync(DOCS_PATH);
	if (updated === current) {
		log("docs already up to date");
		return;
	}
	if (check) {
		console.error(
			"docs/src/benchmarks.md is out of date; rerun `devenv shell -- benchmark:docs` and commit the result",
		);
		Deno.exit(1);
	}
	await Deno.writeTextFile(DOCS_PATH, updated);
	log(`rewrote generated tables in ${DOCS_PATH}`);
}

await main();
