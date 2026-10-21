// The build-speed benchmark: hyperfine over the same fixture app built by
// different tools, or by the base and head builds of the same tool.
//
// Scenarios:
// - `warm` — a no-change rebuild after a priming build. This is where the
//   leptoskit pipeline's content-fingerprinted wasm-bindgen skip shows.
// - `incremental` — a client source bump before every run: cargo recompiles
//   the client, and the pipeline must re-bind (and in release, re-optimize)
//   the changed wasm. This is the loop developers live in.
// - `cold` — the project target directory and output removed: a full
//   compile. Off by default; it is the headline number of the static
//   external comparison, not something a PR needs to re-prove.
//
// Usage (self comparison, base vs head CLI):
//   deno run -A benchmarks/scripts/build_bench.ts \
//     --out build.json --scenarios warm,incremental \
//     --tool "base|apps/runtime/leptoskit|/path/base-cli build --release --out dist-bench" \
//     --tool "head|apps/runtime/leptoskit|/path/head-cli build --release --out dist-bench"
//
// Usage (external comparison) via `benchmarks/build.external.json`:
//   deno run -A benchmarks/scripts/build_bench.ts \
//     --out build.json --manifest benchmarks/build.external.json \
//     --scenarios warm,incremental,cold --cli /path/to/leptoskit

import {
	aggregate,
	type BenchmarkRun,
	BENCHMARKS_ROOT,
	type CliArguments,
	emptyMetric,
	fail,
	log,
	optionalNumber,
	optionalString,
	parseArguments,
	requireString,
} from "./common.ts";

interface Tool {
	name: string;
	/** Fixture root, relative to the benchmarks directory. */
	fixture: string;
	/** The shell command; `{cli}` is replaced with the `--cli` binary. */
	command: string;
}

interface ExternalManifest {
	tools: Tool[];
}

/** Wall-clock samples for one hyperfine command, in milliseconds. */
interface HyperfineResult {
	name: string;
	timesMs: number[];
}

// hyperfine 1.x exports `{ command, median, times }`; 2.x exports
// `{ command, name, measurements[].time_wall_clock, summary }`. Both report
// seconds. Normalising here keeps the rest of the script schema-agnostic.
function parseHyperfineResult(raw: Record<string, unknown>): HyperfineResult {
	const name = typeof raw.name === "string"
		? raw.name
		: String(raw.command ?? "");
	let seconds: number[] = [];
	if (Array.isArray(raw.times)) {
		seconds = raw.times as number[];
	} else if (Array.isArray(raw.measurements)) {
		seconds = (raw.measurements as { time_wall_clock?: { value: number } }[])
			.map((measurement) => measurement.time_wall_clock?.value)
			.filter((value): value is number => typeof value === "number");
	}
	return { name, timesMs: seconds.map((value) => value * 1000) };
}

// The incremental scenario rewrites the fixture's rendered `BENCH_STAMP`
// constant with a random value before every run: the client recompiles and
// the wasm module genuinely changes, so wasm-bindgen (and wasm-opt in
// release) run too — the path a real source edit takes. A comment edit would
// leave the wasm byte-identical and the pipeline would skip binding. Plain
// `sh` only: hyperfine runs it, and `$RANDOM` is not POSIX.
const STAMP_PREPARE =
	`stamp=$(head -c 8 /dev/urandom | od -An -tu8 | tr -d ' ') && sed -i.bak -E "s/stamp-[0-9]+/stamp-$stamp/" client/src/lib.rs && rm -f client/src/lib.rs.bak`;
const STAMP_RESET =
	`sed -i.bak -E "s/stamp-[0-9]+/stamp-0/" client/src/lib.rs && rm -f client/src/lib.rs.bak`;

/** Puts every fixture's stamp back so benchmarking leaves the sources clean. */
function resetStamps(tools: Tool[]): void {
	for (const fixture of new Set(tools.map((tool) => tool.fixture))) {
		new Deno.Command("sh", {
			args: ["-c", STAMP_RESET],
			cwd: fixtureDir(fixture),
		})
			.outputSync();
	}
}
const MEASURED_RUNS = 5;
const COLD_RUNS = 3;

/** Fixture paths may be absolute (CI, `self.ts`) or relative to `benchmarks/`. */
function fixtureDir(fixture: string): string {
	return fixture.startsWith("/") ? fixture : `${BENCHMARKS_ROOT}/${fixture}`;
}

function parseTool(specifier: string): Tool {
	const [name, fixture, command] = specifier.split("|");
	if (!name || !fixture || !command) {
		fail(`--tool expects "name|fixture|command", got: ${specifier}`);
	}
	return { name, fixture, command };
}

/**
 * Runs hyperfine once per command and returns that run's wall-clock time.
 * `--runs 1 --warmup 0`: the caller decides warmups and interleaving, so a
 * scenario's commands alternate (A, B, A, B…) and machine drift spreads
 * over both instead of skewing whichever tool ran last.
 */
async function hyperfineOnce(
	command: { name: string; command: string; cwd: string },
	prepare: string | undefined,
): Promise<number> {
	const exportPath = await Deno.makeTempFile({ suffix: ".json" });
	const args = [
		"--warmup",
		"0",
		"--runs",
		"1",
		"--style",
		"basic",
		"--shell=default",
		"--export-json",
		exportPath,
	];
	if (prepare) {
		args.push("--prepare", `cd ${quote(command.cwd)} && ${prepare}`);
	}
	args.push("--command-name", command.name);
	args.push(`cd ${quote(command.cwd)} && ${command.command}`);
	const status = new Deno.Command("hyperfine", { args, stdin: "inherit" })
		.outputSync();
	if (!status.success) {
		fail(`hyperfine failed: ${new TextDecoder().decode(status.stderr)}`);
	}
	const exported = JSON.parse(await Deno.readTextFile(exportPath)) as {
		results: Record<string, unknown>[];
	};
	await Deno.remove(exportPath);
	const result = exported.results.map(parseHyperfineResult)[0];
	if (!result || result.timesMs.length === 0) {
		fail(`hyperfine recorded no wall-clock sample for ${command.name}`);
	}
	return result.timesMs[0];
}

/**
 * Samples every command once per round, in command order, for `runs` rounds,
 * after one unrecorded warm-up round (unless `options.warmup === 0`, used by
 * the cold scenario, where a warm-up would be a full build by another name).
 */
async function runHyperfine(
	commands: { name: string; command: string; cwd: string }[],
	options: { prepare?: string; runs: number; warmup?: number },
): Promise<Map<string, HyperfineResult>> {
	if ((options.warmup ?? 1) > 0) {
		for (const command of commands) {
			await hyperfineOnce(command, options.prepare);
		}
	}
	const samples = new Map<string, number[]>(
		commands.map((command) => [command.name, []]),
	);
	for (let round = 0; round < options.runs; round += 1) {
		for (const command of commands) {
			samples.get(command.name)!.push(
				await hyperfineOnce(command, options.prepare),
			);
		}
		log(`sampled round ${round + 1}/${options.runs}`);
	}
	return new Map(
		[...samples].map(([name, timesMs]) => [name, { name, timesMs }]),
	);
}

function quote(value: string): string {
	return `"${value.replaceAll('"', '\\"')}"`;
}

function prime(tools: Tool[], cli: string | undefined): void {
	for (const tool of tools) {
		const cwd = fixtureDir(tool.fixture);
		const command = tool.command.replaceAll("{cli}", cli ?? "");
		log(`priming ${tool.name} in ${cwd}`);
		const status = new Deno.Command("bash", {
			args: ["-c", command],
			cwd,
		}).outputSync();
		if (!status.success) {
			fail(
				`priming ${tool.name} failed:\n${
					new TextDecoder().decode(status.stderr)
				}`,
			);
		}
	}
}

async function verifySharedSources(tools: Tool[]): Promise<void> {
	const fixtures = [...new Set(tools.map((tool) => tool.fixture))];
	if (fixtures.length < 2) return;
	const sources = ["client/src/lib.rs", "src/main.rs"];
	for (const source of sources) {
		const contents: string[] = [];
		for (const fixture of fixtures) {
			try {
				contents.push(
					await Deno.readTextFile(`${fixtureDir(fixture)}/${source}`),
				);
			} catch {
				fail(`fixture ${fixture} is missing ${source}`);
			}
		}
		if (new Set(contents).size !== 1) {
			fail(
				`${source} differs between fixtures ${
					fixtures.join(" and ")
				}: the build-speed comparison requires identical app sources`,
			);
		}
	}
}

async function main(): Promise<void> {
	const args: CliArguments = parseArguments(Deno.args);
	const out = requireString(args, "out");
	const scenarioNames =
		(optionalString(args, "scenarios") ?? "warm,incremental")
			.split(",")
			.map((scenario) => scenario.trim());
	const cli = optionalString(args, "cli");
	const runs = optionalNumber(args, "runs") ?? MEASURED_RUNS;
	const coldRuns = optionalNumber(args, "cold-runs") ?? COLD_RUNS;

	let tools: Tool[] = [];
	const manifestPath = optionalString(args, "manifest");
	if (manifestPath) {
		const manifest = JSON.parse(
			await Deno.readTextFile(manifestPath),
		) as ExternalManifest;
		tools = manifest.tools;
	}
	const toolSpecifiers = args.flags.get("tool");
	if (typeof toolSpecifiers !== "undefined") {
		const specifiers = Array.isArray(toolSpecifiers)
			? toolSpecifiers
			: [String(toolSpecifiers)];
		tools.push(...specifiers.map((specifier) => parseTool(specifier)));
	}
	if (tools.length === 0) {
		fail("no tools: pass --tool or --manifest");
	}
	await verifySharedSources(tools);
	await prime(tools, cli);

	// The stamps go back however the run ends: `fail()` exits the process,
	// which skips `finally` blocks but still fires `unload`.
	globalThis.addEventListener("unload", () => resetStamps(tools));

	const metrics: Record<string, ReturnType<typeof emptyMetric>> = {};
	for (const scenario of scenarioNames) {
		const commands = tools.map((tool) => ({
			name: `${tool.name} · ${scenario}`,
			command: tool.command.replaceAll("{cli}", cli ?? ""),
			cwd: fixtureDir(tool.fixture),
		}));
		const options = scenario === "cold"
			// Cold runs get no warmup: `--prepare` removes the target
			// directory, so a warmup run would both pay the full compile and
			// re-execute the prepare. Every measured run starts from scratch.
			? { prepare: "rm -rf target dist-bench", runs: coldRuns, warmup: 0 }
			: scenario === "incremental"
			? { prepare: STAMP_PREPARE, runs }
			: { runs };
		if (scenario === "cold") {
			// A cold run needs a warm priming state for its `--prepare` to be
			// the only difference between runs.
			await prime(tools, cli);
		}
		const results = await runHyperfine(commands, options);
		for (const tool of tools) {
			const result = results.get(`${tool.name} · ${scenario}`);
			if (!result) {
				fail(`hyperfine produced no result for ${tool.name} · ${scenario}`);
			}
			const id = `build.${scenario}.${tool.name}`;
			metrics[id] = emptyMetric("ms", "build", `${scenario} build`);
			if (result.timesMs.length === 0) {
				fail(
					`hyperfine recorded no wall-clock samples for ${tool.name} · ${scenario}`,
				);
			}
			metrics[id].values.push(...result.timesMs);
		}
	}

	const run: BenchmarkRun = aggregate({
		subject: "build",
		environment: {
			browser: "n/a",
			platform: `${Deno.build.os} ${Deno.build.arch}`,
			cpus: navigator.hardwareConcurrency,
		},
		runs: 1,
		metrics,
	});
	await Deno.writeTextFile(out, JSON.stringify(run, null, "\t") + "\n");
	for (const [id, metric] of Object.entries(run.metrics)) {
		log(`${id}: ${metric.median.toFixed(0)} ms`);
	}
}

await main();
