// The full self comparison, mirroring what the CI workflow runs on every
// pull request: build the base and head CLIs from two worktrees, build the
// fixture with each, run the browser runtime benchmark against both dists,
// hyperfine the build pipeline for both binaries, and print the comparison.
//
// Locally, base defaults to `origin/main` in a temporary worktree:
//   devenv shell -- benchmark:self
//
// The CI workflow calls the underlying scripts directly with its own
// worktrees; this script exists so the same comparison runs on a laptop.
//
// Environment:
//   LEPTOSKIT_BENCH_BASE   a commit-ish for the base (default origin/main)

import {
	BENCHMARKS_ROOT,
	type CliArguments,
	fail,
	log,
	parseArguments,
	REPO_ROOT,
	requireString,
	RUNTIME_APPS_ROOT,
	syncBenchGlue,
} from "./common.ts";

function run(command: string, cwd?: string): void {
	log(`${command}${cwd ? ` (in ${cwd})` : ""}`);
	const status = new Deno.Command("bash", {
		args: ["-c", `set -euo pipefail; ${command}`],
		cwd,
		stdout: "inherit",
		stderr: "inherit",
	}).outputSync();
	if (!status.success) {
		fail(`command failed: ${command}`);
	}
}

async function main(): Promise<void> {
	const args: CliArguments = parseArguments(Deno.args);
	const outDir = requireString(args, "out-dir");
	const baseRef = Deno.env.get("LEPTOSKIT_BENCH_BASE") ?? "origin/main";

	await new Deno.Command("git", {
		args: ["fetch", "origin"],
		cwd: REPO_ROOT,
	})
		.outputSync();

	const worktree = await Deno.makeTempDir({ prefix: "leptoskit-bench-base-" });
	await Deno.remove(worktree);
	const add = new Deno.Command("git", {
		args: ["worktree", "add", "--detach", worktree, baseRef],
		cwd: REPO_ROOT,
	}).outputSync();
	if (!add.success) {
		fail(`could not create the base worktree at ${baseRef}`);
	}

	try {
		await run(
			`cargo build --release --package leptoskit_cli --bin leptoskit`,
			worktree,
		);
		await run(
			`cargo build --release --package leptoskit_cli --bin leptoskit`,
			REPO_ROOT,
		);
		const baseCli = `${worktree}/target/release/leptoskit`;
		const headCli = `${REPO_ROOT}/target/release/leptoskit`;

		// The same fixture source, built by each CLI into its own output.
		const fixture = `${RUNTIME_APPS_ROOT}/leptoskit`;
		await syncBenchGlue([`${fixture}/public`]);
		await run(
			`"${baseCli}" build --release --platform web --out dist-base`,
			fixture,
		);
		await run(
			`"${headCli}" build --release --platform web --out dist-head`,
			fixture,
		);

		await run(
			`deno run -A ${BENCHMARKS_ROOT}/scripts/runtime_bench.ts --app leptoskit --dist base=${fixture}/dist-base --dist head=${fixture}/dist-head --out-dir ${outDir}`,
		);
		await run(
			`deno run -A ${BENCHMARKS_ROOT}/scripts/build_bench.ts --out ${outDir}/build.json --scenarios warm,incremental --tool "base|${fixture}|${baseCli} build --release --out dist-bench-base" --tool "head|${fixture}|${headCli} build --release --out dist-bench-head"`,
		);
		const build = JSON.parse(
			await Deno.readTextFile(`${outDir}/build.json`),
		) as { metrics: Record<string, { median: number }> };
		const baseBuild = {
			schema: 1 as const,
			subject: "build-base",
			timestamp: new Date().toISOString(),
			gitCommit: null,
			environment: { browser: "n/a", platform: "", cpus: null },
			runs: 1,
			metrics: Object.fromEntries(
				Object.entries(build.metrics)
					.filter(([id]) => id.endsWith(".base"))
					.map(([id, metric]) => [id.replace(/\.base$/, ""), metric]),
			),
		};
		const headBuild = {
			...baseBuild,
			subject: "build-head",
			metrics: Object.fromEntries(
				Object.entries(build.metrics)
					.filter(([id]) => id.endsWith(".head"))
					.map(([id, metric]) => [id.replace(/\.head$/, ""), metric]),
			),
		};
		await Deno.writeTextFile(
			`${outDir}/base-build.json`,
			JSON.stringify(baseBuild, null, "\t"),
		);
		await Deno.writeTextFile(
			`${outDir}/head-build.json`,
			JSON.stringify(headBuild, null, "\t"),
		);

		await run(
			`deno run -A ${BENCHMARKS_ROOT}/scripts/compare.ts --base-runtime ${outDir}/base-runtime.json --head-runtime ${outDir}/head-runtime.json --base-build ${outDir}/base-build.json --head-build ${outDir}/head-build.json --output ${outDir}/comment.md`,
		);
	} finally {
		await new Deno.Command("git", {
			args: ["worktree", "remove", "--force", worktree],
			cwd: REPO_ROOT,
		}).outputSync();
	}
}

await main();
