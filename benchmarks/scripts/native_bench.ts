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
// `--repo` points the build at another checkout (the base worktree of a
// pull request); the binary is still run from here. Device builds generate
// QuickJS bindings with bindgen, which needs a host libclang: Xcode's on
// macOS, the system LLVM's on Linux (`LIBCLANG_PATH` overrides discovery).

import {
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
	run(
		[
			"cargo",
			"build",
			"--release",
			"--target",
			"wasm32-unknown-unknown",
			"--package",
			"leptoskit_bench_guest",
		],
		{ cwd: repo, env: cargoEnv },
	);
	const guest =
		`${targetDir}/wasm32-unknown-unknown/release/leptoskit_bench_guest.wasm`;
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
		env: { ...cargoEnv, LEPTOSKIT_BENCH_GUEST_WASM: guest },
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
