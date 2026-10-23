// The size benchmark: what a minimal app weighs on every publishable
// platform.
//
// Every platform is scaffolded fresh — the web app through `leptoskit new`,
// the native app through `leptoskit new --platform native` — built in release
// with the CLI under test, and measured by walking the artifact the platform
// actually ships: the web client bundle (raw and brotli, as served), the
// macOS `.app`, the iOS simulator `.app`, and the Android APK.
//
// Usage:
//   deno run -A benchmarks/scripts/size_bench.ts \
//     --cli target/release/leptoskit \
//     [--platforms web,macos,ios-simulator,android] \
//     [--out benchmarks/results/sizes.json] [--subject sizes] [--keep]
//
// Android builds need ANDROID_HOME with an NDK; Apple builds need Xcode.
// A platform whose toolchain is missing is skipped with a note.

import { brotliCompressSync } from "node:zlib";
import {
	aggregate,
	type BenchmarkRun,
	type CliArguments,
	fail,
	log,
	type Metric,
	type MetricClass,
	type MetricUnit,
	optionalString,
	parseArguments,
	requireString,
} from "./common.ts";

type Platform =
	| "web"
	| "macos"
	| "ios-simulator"
	| "android"
	| "android-x86_64";

interface MetricDraft {
	unit: MetricUnit;
	class: MetricClass;
	label: string;
	values: number[];
}

const NATIVE_TARGETS: Record<string, string> = {
	"macos": "macos",
	"ios-simulator": "ios-simulator",
	"android": "android",
	"android-x86_64": "android-x86_64",
};

/** The toolchain the repository pins, so scaffolded projects build with the
 * same one the SDK does rather than the machine's default. */
function pinnedToolchain(): string | null {
	try {
		const file = new URL("../../rust-toolchain.toml", import.meta.url).pathname;
		const match = Deno.readTextFileSync(file).match(/channel\s*=\s*"([^"]+)"/);
		return match?.[1] ?? null;
	} catch {
		return null;
	}
}

function run(
	command: string,
	cwd: string,
	environment: Record<string, string> = {},
): void {
	log(`${command} (in ${cwd})`);
	const status = new Deno.Command("bash", {
		args: ["-c", `set -euo pipefail; ${command}`],
		cwd,
		env: { ...Deno.env.toObject(), ...environment },
		stdout: "inherit",
		stderr: "inherit",
	}).outputSync();
	if (!status.success) {
		fail(`command failed: ${command}`);
	}
}

function commandExists(name: string): boolean {
	return new Deno.Command("bash", {
		args: ["-c", `command -v ${name}`],
	}).outputSync().success;
}

/** Every file under `directory`, as `[relative path, bytes]`. */
function walkFiles(directory: string): [string, number][] {
	const files: [string, number][] = [];
	function visit(path: string, prefix: string) {
		for (const entry of Deno.readDirSync(path)) {
			const child = `${path}/${entry.name}`;
			if (entry.isDirectory) visit(child, `${prefix}${entry.name}/`);
			else if (entry.isFile) {
				files.push([`${prefix}${entry.name}`, Deno.statSync(child).size]);
			}
		}
	}
	visit(directory, "");
	return files;
}

function add(
	metrics: Record<string, MetricDraft>,
	id: string,
	label: string,
	value: number,
): void {
	const existing = metrics[id] ??
		{ unit: "bytes" as const, class: "bytes" as const, label, values: [] };
	if (existing.values.length === 0) existing.values.push(value);
	metrics[id] = existing;
}

/** The brotli size of a file: its precompressed `.br` sibling when the build
 * wrote one, computed at the CLI's settings otherwise. */
function brotliSize(path: string): number {
	const compressed = `${path}.br`;
	try {
		return Deno.statSync(compressed).isFile
			? Deno.statSync(compressed).size
			: brotliCompressSync(Deno.readFileSync(path)).length;
	} catch {
		return brotliCompressSync(Deno.readFileSync(path)).length;
	}
}

function measureWeb(dist: string, metrics: Record<string, MetricDraft>): void {
	const pkg = `${dist}/pkg`;
	const files = walkFiles(pkg).filter(([path]) =>
		path.endsWith(".js") || path.endsWith(".wasm")
	).sort();
	let initialRaw = 0;
	let initialBrotli = 0;
	for (const [path, raw] of files) {
		const brotli = brotliSize(`${pkg}/${path}`);
		initialRaw += raw;
		initialBrotli += brotli;
		const kind = path.endsWith(".wasm") ? "wasm" : "js";
		add(metrics, `web.${kind}`, `web client ${kind}, raw`, raw);
		add(metrics, `web.${kind}.brotli`, `web client ${kind}, brotli`, brotli);
	}
	add(metrics, "web.initial.raw", "web initial load, raw", initialRaw);
	add(
		metrics,
		"web.initial.brotli",
		"web initial load, brotli as served",
		initialBrotli,
	);
}

function measureAppBundle(
	appDir: string,
	prefix: string,
	metrics: Record<string, MetricDraft>,
): number {
	const files = walkFiles(appDir);
	let total = 0;
	for (const [path, bytes] of files) {
		total += bytes;
		if ((path === "App" || path === "Contents/MacOS/App")) {
			add(metrics, `${prefix}.binary`, `${prefix} application binary`, bytes);
		} else if (path.endsWith("app.wasm")) {
			add(metrics, `${prefix}.wasm`, `${prefix} guest wasm asset`, bytes);
		} else if (path.endsWith("app.lkb")) {
			add(metrics, `${prefix}.lkb`, `${prefix} code push baseline`, bytes);
		}
	}
	add(metrics, `${prefix}.total`, `${prefix} application total`, total);
	return total;
}

/** The largest entries inside an APK, from its central directory. */
function measureApk(
	apk: string,
	prefix: string,
	metrics: Record<string, MetricDraft>,
): number {
	const listing = new TextDecoder().decode(
		new Deno.Command("unzip", { args: ["-l", apk] }).outputSync().stdout,
	);
	let nativeLib = 0;
	let assets = 0;
	for (const line of listing.split("\n")) {
		// `unzip -l` prints "  <length>  <MM-DD-YYYY>  <HH:MM>   <name>".
		const match = line.match(/^\s*(\d+)\s+\S+\s+\S+\s+(.+?)\s*$/);
		if (!match) continue;
		const [, length, name] = match;
		const size = Number(length);
		if (name.startsWith("lib/") && name.endsWith(".so")) nativeLib += size;
		if (name.startsWith("assets/")) {
			assets += size;
			if (name === "assets/app.wasm") {
				add(metrics, `${prefix}.wasm`, `${prefix} guest wasm asset`, size);
			} else if (name === "assets/app.lkb") {
				add(metrics, `${prefix}.lkb`, `${prefix} code push baseline`, size);
			}
		}
	}
	add(
		metrics,
		`${prefix}.apk`,
		`${prefix} APK, as downloaded`,
		Deno.statSync(apk).size,
	);
	if (nativeLib > 0) {
		add(
			metrics,
			`${prefix}.native-lib`,
			`${prefix} native library, uncompressed`,
			nativeLib,
		);
	}
	if (assets > 0) {
		add(
			metrics,
			`${prefix}.assets`,
			`${prefix} bundled assets, uncompressed`,
			assets,
		);
	}
	return Deno.statSync(apk).size;
}

/** The newest `.app` under an xcodebuild products directory. */
function newestApp(products: string): string | null {
	let newest: { path: string; modified: number } | null = null;
	try {
		for (const entry of Deno.readDirSync(products)) {
			if (!entry.isDirectory || !entry.name.startsWith("Release")) continue;
			const app = `${products}/${entry.name}/App.app`;
			try {
				const modified = Deno.statSync(app).mtime?.getTime() ?? 0;
				if (!newest || modified > newest.modified) {
					newest = { path: app, modified };
				}
			} catch { /* configuration directory without an app */ }
		}
	} catch {
		return null;
	}
	return newest?.path ?? null;
}

async function main(): Promise<void> {
	const args: CliArguments = parseArguments(Deno.args);
	const cli = requireString(args, "cli");
	const platforms = String(
		args.flags.get("platforms") ?? "web,macos,ios-simulator,android",
	).split(",").map((platform) => platform.trim()).filter(Boolean);
	const out = optionalString(args, "out");
	const subject = optionalString(args, "subject") ?? "sizes";
	const keep = args.flags.get("keep") === true;

	const notes: string[] = [];
	const metrics: Record<string, MetricDraft> = {};
	const workDir = await Deno.makeTempDir({ prefix: "leptoskit-size-" });
	const sdk = new URL("../../", import.meta.url).pathname.replace(/\/$/, "");
	const environment: Record<string, string> = {};
	const toolchain = pinnedToolchain();
	if (toolchain) environment.RUSTUP_TOOLCHAIN = toolchain;

	try {
		for (const platform of platforms) {
			log(`scaffolding and building ${platform}`);
			const project = `${workDir}/${platform.replace(/-/g, "_")}`;
			if (platform === "web") {
				run(
					`"${cli}" new --platform web "${project}" --sdk-path "${sdk}"`,
					workDir,
					environment,
				);
				run(
					`"${cli}" build --release --platform web --out dist`,
					project,
					environment,
				);
				measureWeb(`${project}/dist`, metrics);
			} else if (platform in NATIVE_TARGETS) {
				const target = NATIVE_TARGETS[platform];
				const platformEnvironment = { ...environment };
				if (platform.startsWith("android")) {
					const home = Deno.env.get("ANDROID_HOME") ??
						Deno.env.get("ANDROID_SDK_ROOT");
					if (!home) {
						notes.push("android skipped: ANDROID_HOME is not set");
						continue;
					}
					platformEnvironment.ANDROID_HOME = home;
				}
				if (
					(platform === "macos" || platform === "ios-simulator") &&
					!commandExists("xcodebuild")
				) {
					notes.push(`${platform} skipped: xcodebuild is not available`);
					continue;
				}
				run(
					`"${cli}" new --platform native "${project}" --sdk-path "${sdk}"`,
					workDir,
					platformEnvironment,
				);
				run(
					`"${cli}" native build ${target} --release`,
					project,
					platformEnvironment,
				);
				if (platform === "macos") {
					measureAppBundle(
						`${project}/.leptoskit/macos/App.app`,
						"macos",
						metrics,
					);
				} else if (platform === "ios-simulator") {
					const app = newestApp(
						`${project}/.leptoskit/ios/build/Build/Products`,
					);
					if (app) measureAppBundle(app, "ios", metrics);
					else notes.push("ios-simulator: no .app found to measure");
				} else {
					const outputs =
						`${project}/hosts/android/app/build/outputs/apk/release`;
					const apks = walkFiles(outputs).filter(([path]) =>
						path.endsWith(".apk")
					);
					if (apks.length === 0) {
						notes.push(`${platform}: no APK found to measure`);
						continue;
					}
					measureApk(`${outputs}/${apks[0][0]}`, "android", metrics);
				}
			} else {
				fail(`unknown platform ${platform}`);
			}
		}
	} finally {
		if (keep) log(`keeping ${workDir}`);
		else await Deno.remove(workDir, { recursive: true }).catch(() => {});
	}

	const draft: Parameters<typeof aggregate>[0] = {
		subject,
		environment: {
			browser: "n/a",
			platform: `${Deno.build.os} ${Deno.build.arch}`,
			cpus: navigator.hardwareConcurrency,
		},
		runs: 1,
		metrics,
	};
	const aggregated: BenchmarkRun = { ...aggregate(draft), notes };
	if (out) {
		await Deno.writeTextFile(out, JSON.stringify(aggregated, null, "\t"));
		log(`wrote ${out}`);
	}
	for (const [id, metric] of Object.entries(aggregated.metrics)) {
		log(`${id}: ${(metric as Metric).median} bytes`);
	}
	for (const note of notes) log(note);
}

await main();
