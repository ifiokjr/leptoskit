// Runs the external comparison: every runtime fixture present (leptoskit
// plus the competition) and the build-speed face-off, writing the two JSON
// results files the documentation tables are generated from.
//
// These numbers are static reference data, not CI gates: rerun with the
// `benchmark:docs` task (or this script with --docs) when dependencies or
// the framework itself move, and commit the refreshed tables.
//
// Usage:
//   deno run -A benchmarks/scripts/external.ts \
//     --cli /path/to/leptoskit \
//     --runtime-out benchmarks/results/external-runtime.json \
//     --build-out benchmarks/results/external-build.json \
//     [--runs 7] [--skip-build] [--skip-flutter] [--docs]

import {
	type BenchmarkRun,
	BENCHMARKS_ROOT,
	type CliArguments,
	fail,
	log,
	optionalNumber,
	parseArguments,
	requireString,
	RUNTIME_APPS_ROOT,
	syncBenchGlue,
} from "./common.ts";

const STATIC_APPS = ["react", "react-native-web", "vanilla-js"];

async function pathExists(path: string): Promise<boolean> {
	try {
		await Deno.stat(path);
		return true;
	} catch {
		return false;
	}
}

async function buildLeptoskitApp(cli: string, out: string): Promise<void> {
	const fixture = `${RUNTIME_APPS_ROOT}/leptoskit`;
	await syncBenchGlue([`${fixture}/public`]);
	log(`building leptoskit fixture with ${cli}`);
	const status = new Deno.Command("bash", {
		args: ["-c", `"${cli}" build --release --platform web --out dist`],
		cwd: fixture,
	}).outputSync();
	if (!status.success) {
		fail(
			`leptoskit fixture build failed:\n${
				new TextDecoder().decode(status.stderr)
			}`,
		);
	}
	void out;
}

async function buildFlutterApp(): Promise<boolean> {
	const fixture = `${RUNTIME_APPS_ROOT}/flutter`;
	const which = new Deno.Command("which", { args: ["flutter"] }).outputSync();
	if (!which.success) {
		log("flutter not found; skipping the flutter web app");
		return false;
	}
	await syncBenchGlue([`${fixture}/web`]);
	log("building the flutter web app");
	const status = new Deno.Command("flutter", {
		args: ["build", "web", "--release"],
		cwd: fixture,
	}).outputSync();
	if (!status.success) {
		fail(
			`flutter web build failed:\n${new TextDecoder().decode(status.stderr)}`,
		);
	}
	return true;
}

function runRuntimeBench(
	app: string,
	dist: string,
	out: string,
	runs: number,
): boolean {
	log(`runtime benchmark: ${app}`);
	const command = new Deno.Command(Deno.execPath(), {
		args: [
			"run",
			"-A",
			`${BENCHMARKS_ROOT}/scripts/runtime_bench.ts`,
			"--app",
			app,
			"--dist",
			dist,
			"--out",
			out,
			"--runs",
			String(runs),
		],
	}).outputSync();
	if (!command.success) {
		console.log(new TextDecoder().decode(command.stdout));
		console.error(new TextDecoder().decode(command.stderr));
		return false;
	}
	return true;
}

async function main(): Promise<void> {
	const args: CliArguments = parseArguments(Deno.args);
	const cli = requireString(args, "cli");
	const runtimeOut = requireString(args, "runtime-out");
	const buildOut = requireString(args, "build-out");
	const runs = optionalNumber(args, "runs") ?? 7;
	const skipBuild = args.flags.get("skip-build") === true;
	const withDocs = args.flags.get("docs") === true;

	const runtimeRuns: BenchmarkRun[] = [];

	// The static fixtures run whenever their vendor bundles are present.
	for (const app of STATIC_APPS) {
		if (!await pathExists(`${RUNTIME_APPS_ROOT}/${app}`)) continue;
		const vendor = app === "vanilla-js"
			? true
			: await Deno.stat(`${RUNTIME_APPS_ROOT}/${app}/vendor`)
				.then(() => true)
				.catch(() => false);
		if (!vendor) {
			log(
				`${app} has no vendor bundle; run scripts/vendor.sh first (skipping)`,
			);
			continue;
		}
		const out = await Deno.makeTempFile({ suffix: ".json" });
		if (
			await runRuntimeBench(app, `${RUNTIME_APPS_ROOT}/${app}`, out, runs)
		) {
			runtimeRuns.push(
				JSON.parse(await Deno.readTextFile(out)) as BenchmarkRun,
			);
		}
		await Deno.remove(out);
	}

	if (!skipBuild) {
		await buildLeptoskitApp(cli, `${RUNTIME_APPS_ROOT}/leptoskit/dist`);
	}
	const out = await Deno.makeTempFile({ suffix: ".json" });
	if (
		await runRuntimeBench(
			"leptoskit",
			`${RUNTIME_APPS_ROOT}/leptoskit/dist`,
			out,
			runs,
		)
	) {
		runtimeRuns.push(JSON.parse(await Deno.readTextFile(out)) as BenchmarkRun);
	}
	await Deno.remove(out);

	if (
		args.flags.get("skip-flutter") !== true &&
		await pathExists(`${RUNTIME_APPS_ROOT}/flutter/pubspec.yaml`)
	) {
		if (await buildFlutterApp()) {
			const out = await Deno.makeTempFile({ suffix: ".json" });
			if (
				await runRuntimeBench(
					"flutter",
					`${RUNTIME_APPS_ROOT}/flutter/build/web`,
					out,
					runs,
				)
			) {
				runtimeRuns.push(
					JSON.parse(await Deno.readTextFile(out)) as BenchmarkRun,
				);
			}
			await Deno.remove(out);
		}
	}

	if (runtimeRuns.length === 0) {
		fail("no runtime benchmark completed");
	}
	const merged: BenchmarkRun = {
		schema: 1,
		subject: "external-runtime",
		timestamp: new Date().toISOString(),
		gitCommit: runtimeRuns[0].gitCommit,
		environment: runtimeRuns[0].environment,
		runs,
		// Merge by metric id; each app contributes its own medians, and apps
		// that cannot run a metric (canvas-rendered Flutter) simply lack it.
		metrics: Object.fromEntries(
			runtimeRuns.flatMap((run) =>
				Object.entries(run.metrics).map(([id, metric]) => [
					`${run.subject}.${id}`,
					metric,
				])
			),
		),
	};
	await Deno.writeTextFile(
		runtimeOut,
		JSON.stringify(merged, null, "\t") + "\n",
	);
	log(`wrote ${runtimeOut} (${runtimeRuns.length} apps)`);

	if (!skipBuild) {
		const buildCommand = new Deno.Command(Deno.execPath(), {
			args: [
				"run",
				"-A",
				`${BENCHMARKS_ROOT}/scripts/build_bench.ts`,
				"--out",
				buildOut,
				"--manifest",
				`${BENCHMARKS_ROOT}/build.external.json`,
				"--scenarios",
				"warm,incremental,cold",
				"--cli",
				cli,
			],
		}).outputSync();
		if (!buildCommand.success) {
			fail(
				`build benchmark failed:\n${
					new TextDecoder().decode(buildCommand.stderr)
				}`,
			);
		}
		log(`wrote ${buildOut}`);
	}

	if (withDocs) {
		const docs = new Deno.Command(Deno.execPath(), {
			args: [
				"run",
				"-A",
				`${BENCHMARKS_ROOT}/scripts/docs_update.ts`,
				"--runtime",
				runtimeOut,
				"--build",
				buildOut,
			],
		}).outputSync();
		if (!docs.success) {
			fail(
				`docs update failed:\n${new TextDecoder().decode(docs.stderr)}`,
			);
		}
	}
}

await main();
