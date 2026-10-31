// The native-layer benchmark harness: builds the wasm guest and the
// `leptoskit_bench` binary for the host or a device target, runs it — on the
// host directly, on an iOS simulator or Android emulator through
// `leptoskit test run` — and writes one BenchmarkRun JSON.
//
// Usage:
//   deno run -A benchmarks/scripts/native_bench.ts --out run.json \
//     [--repo <checkout>] [--platform host|ios|android] [--device <udid|serial>] \
//     [--cli <leptoskit binary>] [--runs 15] [--warmup 2] [--subject native-host] \
//     [--android-ndk <dir>] [--android-arch aarch64|x86_64]
//
// Comparing two checkouts on the host:
//   deno run -A benchmarks/scripts/native_bench.ts --out head.json --subject head \
//     --base-repo <base checkout> --base-out base.json [--rounds 5]
//
// A comparison builds both binaries before measuring either, then runs them
// in alternating rounds (head, base, base, head, ...) and joins each side's
// samples. Measured one after the other, the two sides land in different
// windows of a shared runner whose speed drifts: identical code read 28%
// slower on one side of a pull request, while one runner measured the same
// binary 6–10% apart a minute later and another runner 30% faster.
//
// `--repo` points the build at another checkout (the base worktree of a
// pull request); the binary is still run from here. Device builds generate
// QuickJS bindings with bindgen, which needs a host libclang: Xcode's on
// macOS, the system LLVM's on Linux (`LIBCLANG_PATH` overrides discovery).

import { existsSync } from "node:fs";

import {
	aggregate,
	type BenchmarkRun,
	type CliArguments,
	fail,
	log,
	optionalNumber,
	optionalString,
	parseArguments,
	REPO_ROOT,
	requireString,
} from "./common.ts";

type Platform = "host" | "ios" | "android";

function run(
	command: string[],
	options: { cwd?: string; env?: Record<string, string> } = {},
): string {
	log(command.join(" "));
	const output = new Deno.Command(command[0], {
		args: command.slice(1),
		cwd: options.cwd,
		env: options.env,
		stderr: "inherit",
	}).outputSync();
	if (!output.success) {
		fail(`command failed: ${command.join(" ")}`);
	}
	return new TextDecoder().decode(output.stdout);
}

function which(binary: string): string | undefined {
	const output = new Deno.Command("which", { args: [binary] }).outputSync();
	return output.success
		? new TextDecoder().decode(output.stdout).trim()
		: undefined;
}

function xcrun(args: string[]): string {
	return run(["xcrun", ...args]).trim();
}

/** Where bindgen finds libclang: an override, then Xcode, then the system LLVM. */
function libclangPath(): string | undefined {
	const override = Deno.env.get("LIBCLANG_PATH");
	if (override) return override;
	const xcode =
		"/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib";
	try {
		if (Deno.statSync(`${xcode}/libclang.dylib`).isFile) return xcode;
	} catch {
		// not macOS, or no Xcode
	}
	const config = which("llvm-config");
	if (config) {
		const output = new Deno.Command(config, { args: ["--libdir"] })
			.outputSync();
		if (output.success) return new TextDecoder().decode(output.stdout).trim();
	}
	for (
		const candidate of [
			"/usr/lib/llvm-18/lib",
			"/usr/lib/llvm-17/lib",
			"/usr/lib/llvm-16/lib",
		]
	) {
		try {
			Deno.statSync(candidate);
			return candidate;
		} catch {
			// next
		}
	}
	return undefined;
}

interface Target {
	triple: string;
	env: Record<string, string>;
}

function hostTarget(): Target {
	return { triple: "", env: {} };
}

function iosTarget(): Target {
	const simulatorSdk = xcrun(["--sdk", "iphonesimulator", "--show-sdk-path"]);
	const env: Record<string, string> = {
		SDKROOT: xcrun(["--sdk", "macosx", "--show-sdk-path"]),
		CARGO_TARGET_AARCH64_APPLE_IOS_SIM_RUSTFLAGS:
			`-C link-arg=-isysroot${simulatorSdk}`,
	};
	const libclang = libclangPath();
	if (libclang) env.LIBCLANG_PATH = libclang;
	return { triple: "aarch64-apple-ios-sim", env };
}

function androidTarget(ndk: string, arch: "aarch64" | "x86_64"): Target {
	const hostTag = Deno.build.os === "darwin" ? "darwin-x86_64" : "linux-x86_64";
	const toolchain = `${ndk}/toolchains/llvm/prebuilt/${hostTag}`;
	const triple = `${arch}-linux-android`;
	const clang = `${toolchain}/bin/${arch}-linux-android24-clang`;
	const upper = triple.replaceAll("-", "_").toUpperCase();
	const env: Record<string, string> = {
		[`CARGO_TARGET_${upper}_LINKER`]: clang,
		[`CC_${triple.replaceAll("-", "_")}`]: clang,
		[`AR_${triple.replaceAll("-", "_")}`]: `${toolchain}/bin/llvm-ar`,
		[`BINDGEN_EXTRA_CLANG_ARGS_${triple.replaceAll("-", "_")}`]:
			`--sysroot=${toolchain}/sysroot --target=${triple}24`,
	};
	const libclang = libclangPath();
	if (libclang) env.LIBCLANG_PATH = libclang;
	return { triple, env };
}

function findNdk(explicit: string | undefined): string {
	if (explicit) return explicit;
	const home = Deno.env.get("ANDROID_NDK_HOME") ??
		Deno.env.get("ANDROID_NDK_ROOT");
	if (home) return home;
	const sdk = Deno.env.get("ANDROID_HOME") ??
		Deno.env.get("ANDROID_SDK_ROOT") ??
		`${Deno.env.get("HOME")}/Library/Android/sdk`;
	const versions: string[] = [];
	try {
		for (const entry of Deno.readDirSync(`${sdk}/ndk`)) {
			if (entry.isDirectory) versions.push(entry.name);
		}
	} catch {
		fail("no Android NDK found; pass --android-ndk or set ANDROID_NDK_HOME");
	}
	versions.sort((left, right) =>
		left.localeCompare(right, undefined, { numeric: true })
	);
	return `${sdk}/ndk/${versions[versions.length - 1]}`;
}

/** Builds the guest and the bench in `repo`; returns the bench binary path. */
function build(repo: string, target: Target, targetDir: string): string {
	const cargoEnv = {
		...Deno.env.toObject(),
		CARGO_TARGET_DIR: targetDir,
		...target.env,
	};
	// A base checkout older than the counter guest still builds and measures;
	// its tap scenarios record that the guest is missing.
	const hasCounter = existsSync(`${repo}/benchmarks/native/counter/Cargo.toml`);
	const hasScreen = existsSync(`${repo}/benchmarks/native/screen/Cargo.toml`);
	const guests = ["--package", "leptoskit_bench_guest"];
	if (hasCounter) guests.push("--package", "leptoskit_bench_counter");
	if (hasScreen) guests.push("--package", "leptoskit_bench_screen");
	run(
		[
			"cargo",
			"build",
			"--release",
			"--target",
			"wasm32-unknown-unknown",
			...guests,
		],
		{ cwd: repo, env: cargoEnv },
	);
	const wasmDir = `${targetDir}/wasm32-unknown-unknown/release`;
	const guest = `${wasmDir}/leptoskit_bench_guest.wasm`;
	const embedded: Record<string, string> = {
		LEPTOSKIT_BENCH_GUEST_WASM: guest,
	};
	if (hasCounter) {
		embedded.LEPTOSKIT_BENCH_COUNTER_WASM =
			`${wasmDir}/leptoskit_bench_counter.wasm`;
	}
	if (hasScreen) {
		embedded.LEPTOSKIT_BENCH_SCREEN_WASM =
			`${wasmDir}/leptoskit_bench_screen.wasm`;
	}
	const command = [
		"cargo",
		"build",
		"--release",
		"--package",
		"leptoskit_bench",
	];
	if (target.triple) command.push("--target", target.triple);
	run(command, {
		cwd: repo,
		env: { ...cargoEnv, ...embedded },
	});
	const profileDir = target.triple
		? `${targetDir}/${target.triple}/release`
		: `${targetDir}/release`;
	return `${profileDir}/leptoskit_bench`;
}

const JSON_BEGIN = "LEPTOSKIT_BENCH_JSON_BEGIN";
const JSON_END = "LEPTOSKIT_BENCH_JSON_END";

function extractJson(stdout: string): string {
	const begin = stdout.indexOf(JSON_BEGIN);
	const end = stdout.indexOf(JSON_END);
	if (begin === -1 || end === -1 || end < begin) {
		fail("the benchmark binary printed no JSON report");
	}
	return stdout.slice(begin + JSON_BEGIN.length, end).trim();
}

/** Joins the samples of several runs of one binary into one run. */
function joinRuns(runs: BenchmarkRun[]): BenchmarkRun {
	const metrics: Parameters<typeof aggregate>[0]["metrics"] = {};
	for (const run of runs) {
		for (const [id, metric] of Object.entries(run.metrics)) {
			metrics[id] ??= {
				unit: metric.unit,
				class: metric.class,
				label: metric.label,
				values: [],
			};
			metrics[id].values.push(...metric.values);
		}
	}
	const [first] = runs;
	return {
		...aggregate({
			subject: first.subject,
			environment: first.environment,
			runs: runs.reduce((total, run) => total + run.runs, 0),
			metrics,
		}),
		gitCommit: first.gitCommit,
		notes: first.notes,
	};
}

/**
 * Measures `head` and `base` in alternating rounds of `runs / rounds` samples
 * each, reversing the order every round so a steady drift in the runner's
 * speed lands on both sides alike, and writes each side's joined run.
 */
async function compare(
	sides: { binary: string; subject: string; out: string }[],
	runs: number,
	warmup: number,
	rounds: number,
): Promise<void> {
	const perRound = Math.ceil(runs / rounds);
	const collected = sides.map((): BenchmarkRun[] => []);
	const scratch = await Deno.makeTempDir({ prefix: "leptoskit-bench-pair-" });
	for (let round = 0; round < rounds; round += 1) {
		const order = round % 2 === 0 ? [0, 1] : [1, 0];
		for (const index of order) {
			const side = sides[index];
			const out = `${scratch}/${side.subject}-${round}.json`;
			run([
				side.binary,
				"--runs",
				String(perRound),
				"--warmup",
				String(warmup),
				"--subject",
				side.subject,
				"--out",
				out,
			]);
			collected[index].push(JSON.parse(await Deno.readTextFile(out)));
		}
	}
	for (const [index, side] of sides.entries()) {
		await Deno.writeTextFile(
			side.out,
			JSON.stringify(joinRuns(collected[index]), null, "\t") + "\n",
		);
		log(`wrote ${side.out} (${rounds} interleaved rounds)`);
	}
	await Deno.remove(scratch, { recursive: true });
}

async function main(): Promise<void> {
	const args: CliArguments = parseArguments(Deno.args);
	const out = requireString(args, "out");
	const repo = optionalString(args, "repo") ?? REPO_ROOT;
	const platform = (optionalString(args, "platform") ?? "host") as Platform;
	const runs = optionalNumber(args, "runs") ?? 15;
	const warmup = optionalNumber(args, "warmup") ?? 2;
	const subject = optionalString(args, "subject") ?? `native-${platform}`;
	const device = optionalString(args, "device");
	const cli = optionalString(args, "cli") ??
		`${REPO_ROOT}/target/release/leptoskit`;
	const targetDir = optionalString(args, "target-dir") ?? `${repo}/target`;
	if (platform !== "host" && optionalString(args, "base-repo")) {
		fail("--base-repo compares on the host only");
	}

	const target = platform === "ios"
		? iosTarget()
		: platform === "android"
		? androidTarget(
			findNdk(optionalString(args, "android-ndk")),
			(optionalString(args, "android-arch") as
				| "aarch64"
				| "x86_64"
				| undefined) ?? "aarch64",
		)
		: hostTarget();
	const binary = build(repo, target, targetDir);
	const benchArgs = [
		"--runs",
		String(runs),
		"--warmup",
		String(warmup),
		"--subject",
		subject,
	];

	if (platform === "host") {
		const baseRepo = optionalString(args, "base-repo");
		if (baseRepo) {
			const baseBinary = build(
				baseRepo,
				target,
				optionalString(args, "base-target-dir") ?? `${baseRepo}/target`,
			);
			await compare(
				[
					{ binary, subject, out },
					{
						binary: baseBinary,
						subject: optionalString(args, "base-subject") ?? "base",
						out: requireString(args, "base-out"),
					},
				],
				runs,
				warmup,
				optionalNumber(args, "rounds") ?? 5,
			);
			return;
		}
		run([binary, ...benchArgs, "--out", out]);
		log(`wrote ${out}`);
		return;
	}

	if (!device) {
		fail(`--device is required for --platform ${platform}`);
	}
	const artifacts = await Deno.makeTempDir({
		prefix: "leptoskit-bench-device-",
	});
	run([
		cli,
		"test",
		"run",
		"--platform",
		platform,
		"--device",
		device,
		"--binary",
		binary,
		"--artifacts",
		artifacts,
		"--",
		...benchArgs,
	]);
	// The runner writes one directory per run with the test step's stdout log.
	let stdout = "";
	for (const runDir of Deno.readDirSync(artifacts)) {
		for (const entry of Deno.readDirSync(`${artifacts}/${runDir.name}`)) {
			if (entry.name.endsWith("-test.stdout.log")) {
				stdout += await Deno.readTextFile(
					`${artifacts}/${runDir.name}/${entry.name}`,
				);
			}
		}
	}
	await Deno.writeTextFile(out, extractJson(stdout) + "\n");
	await Deno.remove(artifacts, { recursive: true });
	log(`wrote ${out} (${platform}, device ${device})`);
}

await main();
