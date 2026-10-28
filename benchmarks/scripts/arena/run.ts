// Runs the arena's scenarios: builds each framework's app for a platform,
// installs it, and launches every scenario in interleaved rounds, writing one
// results file per platform.
//
// Usage:
//   deno run -A benchmarks/scripts/arena/run.ts \
//     [--platforms ios,android,macos,web] [--frameworks leptoskit,flutter,...] \
//     [--scenarios cold,echo,...] [--runs 5] [--ios <udid>] [--android <serial>] \
//     [--skip-build] [--allow-busy] [--out-dir benchmarks/results]
//
// A round launches every framework once per scenario, alternating the order
// (A B C D, then D C B A) so drift lands on every framework evenly; one
// unrecorded warm-up round comes first. A platform or framework that cannot
// run is skipped with its reason recorded, never silently.

import {
	BENCHMARKS_ROOT,
	currentCommit,
	fail,
	log,
	optionalNumber,
	optionalString,
	parseArguments,
} from "../common.ts";
import {
	ARENA_APPS_ROOT,
	ARENA_ROOT,
	type ArenaReport,
	type Descriptor,
	type Platform,
	PLATFORMS,
	readDescriptor,
	type Scenario,
	SCENARIOS,
} from "./descriptor.ts";
import {
	frameStats,
	launchToReady,
	locateBeacon,
	phaseWindows,
	type PresentedFrame,
	presentedFrames,
} from "./frames.ts";
import { summarise, type Summary } from "./stats.ts";
import {
	android,
	browser,
	type Capture,
	desktop,
	type DeviceInfo,
	iosSimulator,
	type Target,
} from "./targets.ts";

export const FRAMEWORKS = [
	"leptoskit",
	"react-native",
	"lynx",
	"flutter",
] as const;

/** Scenarios whose numbers come (partly) from the recording. */
const RECORDED: ReadonlySet<Scenario> = new Set([
	"cold",
	"mount",
	"ticker",
	"autoscroll",
	"busy",
]);

const TIMEOUT_MS: Record<Scenario, number> = {
	cold: 30_000,
	mount: 120_000,
	echo: 180_000,
	ticker: 40_000,
	autoscroll: 40_000,
	busy: 40_000,
};

export interface ScenarioResult {
	status: "ok" | "failed" | "unsupported" | "skipped";
	/** Why the scenario failed, is unsupported, or was skipped. */
	reason?: string;
	notes: string[];
	/** Every metric the app reported or the recording yielded, summarised across runs. */
	metrics: Record<string, Summary>;
}

export interface FrameworkResult {
	version: string;
	/** Why every number on this platform needs a caveat (Flutter's debug-only iOS simulator). */
	caveat?: string;
	/** Keyed by scenario, with `.tuned` appended for the tuned lane. */
	scenarios: Record<string, ScenarioResult>;
}

export interface ArenaResults {
	schema: 1;
	subject: string;
	timestamp: string;
	gitCommit: string | null;
	device: DeviceInfo;
	/** The host, and its one-minute load average when the run started. */
	host: { platform: string; cpus: number | null; load: number };
	runs: number;
	frameworks: Record<string, FrameworkResult>;
	skipped: { framework: string; reason: string }[];
}

function list<T extends string>(
	value: string | undefined,
	all: readonly T[],
): T[] {
	if (!value) return [...all];
	const picked = value.split(",").map((item) => item.trim()) as T[];
	for (const item of picked) {
		if (!all.includes(item)) {
			fail(`unknown value ${item}; expected one of ${all.join(", ")}`);
		}
	}
	return picked;
}

// The compiler and linker settings a nix development shell exports. Xcode
// builds read them too: nix's `ld` fails Swift packages' relocatable link,
// and its include and hardening flags break C pods such as PrimJS.
const NIX_TOOLCHAIN = new Set([
	"CC",
	"CXX",
	"LD",
	"AR",
	"AS",
	"NM",
	"RANLIB",
	"STRIP",
	"CONFIG_SHELL",
	"MACOSX_DEPLOYMENT_TARGET",
	"PKG_CONFIG_PATH",
	"cmakeFlags",
]);

/**
 * The environment an Xcode build should see when the arena runs inside the
 * devenv shell: the nix toolchain's variables removed, Xcode's tools first on
 * `PATH`, and the development shell's own tools (wasm-opt, deno) kept after
 * them.
 */
export function xcodeEnvironment(
	environment: Record<string, string> = Deno.env.toObject(),
): Record<string, string> {
	const kept = Object.fromEntries(
		Object.entries(environment).filter(([name]) =>
			!NIX_TOOLCHAIN.has(name) && !name.startsWith("NIX_")
		),
	);
	const entries = (environment.PATH ?? "").split(":").filter(Boolean);
	const store = entries.filter((entry) => entry.startsWith("/nix/store/"));
	const rest = entries.filter((entry) =>
		!entry.startsWith("/nix/store/") && entry !== "/usr/bin"
	);
	kept.PATH = ["/usr/bin", ...rest, ...store].join(":");
	return kept;
}

/** Runs a descriptor command with its placeholders filled. */
export async function shell(
	command: string,
	cwd: string,
	project = cwd,
	platform?: Platform,
): Promise<void> {
	const expanded = command.replaceAll(
		"{versions}",
		`${ARENA_ROOT}/versions.json`,
	).replaceAll("{project}", project);
	log(`${expanded} (in ${cwd})`);
	const apple = platform === "ios" || platform === "macos";
	const status = await new Deno.Command("bash", {
		args: ["-c", `set -euo pipefail; ${expanded}`],
		cwd,
		env: apple ? xcodeEnvironment() : undefined,
		clearEnv: apple,
		stdout: "inherit",
		stderr: "inherit",
	}).output();
	if (!status.success) throw new Error(`command failed: ${expanded}`);
}

function bootedSimulator(): string | undefined {
	const output = new TextDecoder().decode(
		new Deno.Command("xcrun", { args: ["simctl", "list", "devices", "booted"] })
			.outputSync().stdout,
	);
	return output.match(/\(([0-9A-F-]{36})\) \(Booted\)/)?.[1];
}

async function firstAndroidDevice(): Promise<string | undefined> {
	const adb = `${
		Deno.env.get("ANDROID_HOME") ??
			`${Deno.env.get("HOME")}/Library/Android/sdk`
	}/platform-tools/adb`;
	const output = await new Deno.Command(adb, { args: ["devices"] }).output()
		.catch(() => null);
	if (!output?.success) return undefined;
	return new TextDecoder().decode(output.stdout).split("\n").slice(1)
		.map((line) => line.split("\t")).find(([, state]) => state === "device")
		?.[0];
}

async function makeTarget(
	platform: Platform,
	options: { ios?: string; android?: string },
): Promise<Target | string> {
	switch (platform) {
		case "ios": {
			const udid = options.ios ?? bootedSimulator();
			return udid
				? iosSimulator(udid)
				: "no booted iOS simulator (pass --ios <udid>)";
		}
		case "android": {
			const serial = options.android ?? await firstAndroidDevice();
			return serial
				? await android(serial)
				: "no Android device or emulator attached (pass --android <serial>)";
		}
		case "web":
			return browser();
		case "macos":
		case "windows":
		case "linux": {
			const host = {
				darwin: "macos",
				windows: "windows",
				linux: "linux",
			}[Deno.build.os as string];
			return host === platform
				? desktop(platform)
				: `${platform} apps are measured on ${platform}, not ${Deno.build.os}`;
		}
	}
}

/**
 * Why a framework cannot run a scenario on a platform, if it cannot. On the
 * web there is no native layer, so the echo round trip has nothing to cross
 * to; whatever an app measures there (a JavaScript stand-in) is not the
 * scenario.
 */
function unsupportedReason(
	descriptor: Descriptor,
	scenario: Scenario,
	platform: Platform,
): string | undefined {
	if (scenario === "echo" && platform === "web") {
		return "the web has no native layer to call";
	}
	return descriptor.unsupported?.[scenario];
}

/** The artifact to install and the identifier to launch, from a built target. */
function installable(
	descriptor: Descriptor,
	platform: Platform,
): { artifact: string; app: string } | null {
	const root = `${ARENA_APPS_ROOT}/${descriptor.framework}`;
	const targets = descriptor.targets;
	switch (platform) {
		case "ios":
			return targets.ios
				? {
					artifact: `${root}/${targets.ios.simulatorApp}`,
					app: targets.ios.bundleId,
				}
				: null;
		case "android":
			return targets.android
				? {
					artifact: `${root}/${targets.android.apk}`,
					app: targets.android.component,
				}
				: null;
		case "web":
			return targets.web
				? {
					artifact: `${root}/${targets.web.dist}`,
					app: `${root}/${targets.web.dist}`,
				}
				: null;
		case "macos":
		case "windows":
		case "linux": {
			const desktop = targets[platform];
			return desktop
				? {
					artifact: `${root}/${desktop.artifact}`,
					app: `${root}/${desktop.executable}`,
				}
				: null;
		}
	}
}

/** One launch's numbers per report lane: the app's metrics plus what the recording showed. */
async function measure(
	scenario: Scenario,
	capture: Capture,
	refreshHz: number,
): Promise<Map<string, Record<string, number>>> {
	const byLane = new Map<string, Record<string, number>>();
	let frames: PresentedFrame[] | undefined = capture.frames;
	if (!frames && capture.video) {
		frames = await presentedFrames(
			capture.video,
			await locateBeacon(capture.video),
		);
	}
	const windows = frames ? phaseWindows(frames) : [];
	for (const [index, report] of capture.reports.entries()) {
		const metrics: Record<string, number> = { ...report.metrics };
		if (frames) {
			if (scenario === "cold") {
				const ready = launchToReady(frames, capture.launchedAt);
				if (ready !== null) metrics.launch_to_ready_ms = ready;
			} else if (scenario === "mount") {
				// Five rounds of create, update, clear, in order.
				const steps = ["create", "update", "clear"];
				const values: Record<string, number[]> = {};
				for (const [position, window] of windows.entries()) {
					const step = steps[position % steps.length];
					(values[step] ??= []).push((window.end - window.start) * 1000);
				}
				for (const [step, durations] of Object.entries(values)) {
					metrics[`${step}_to_pixel_ms`] = summarise(durations).median;
				}
			} else if (windows[index]) {
				const stats = frameStats(frames, windows[index], refreshHz);
				for (const [key, value] of Object.entries(stats)) {
					metrics[`screen_${key}`] = value;
				}
			}
		}
		if (capture.platformLaunchMs !== undefined) {
			metrics.platform_launch_ms = capture.platformLaunchMs;
		}
		byLane.set(
			report.lane === "tuned" ? `${scenario}.tuned` : scenario,
			metrics,
		);
	}
	if (capture.video) await Deno.remove(capture.video).catch(() => {});
	return byLane;
}

async function main(): Promise<void> {
	const args = parseArguments(Deno.args);
	const platforms = list(optionalString(args, "platforms"), PLATFORMS);
	const frameworks = list(optionalString(args, "frameworks"), FRAMEWORKS);
	const scenarios = list(optionalString(args, "scenarios"), SCENARIOS);
	const runs = optionalNumber(args, "runs") ?? 5;
	const outDir = optionalString(args, "out-dir") ??
		`${BENCHMARKS_ROOT}/results`;
	const skipBuild = args.flags.get("skip-build") === true;

	// A loaded machine inflates every number, and unevenly: refuse to measure
	// on one unless told to, and record the load either way.
	const [load] = Deno.loadavg();
	const cpus = navigator.hardwareConcurrency ?? 1;
	if (load > cpus && args.flags.get("allow-busy") !== true) {
		fail(
			`the load average is ${
				load.toFixed(0)
			} on ${cpus} cores; measure on a quiet machine (or pass --allow-busy)`,
		);
	}

	for (const platform of platforms) {
		const target = await makeTarget(platform, {
			ios: optionalString(args, "ios"),
			android: optionalString(args, "android"),
		});
		if (typeof target === "string") {
			log(`skipping ${platform}: ${target}`);
			continue;
		}
		log(`${platform}: ${target.info.name}`);

		const results: ArenaResults = {
			schema: 1,
			subject: `arena-${platform}`,
			timestamp: new Date().toISOString(),
			gitCommit: currentCommit(),
			device: target.info,
			host: {
				platform: `${Deno.build.os} ${Deno.build.arch}`,
				cpus: navigator.hardwareConcurrency ?? null,
				load,
			},
			runs,
			frameworks: {},
			skipped: [],
		};
		const ready: { framework: string; descriptor: Descriptor; app: string }[] =
			[];
		for (const framework of frameworks) {
			let descriptor: Descriptor;
			try {
				descriptor = readDescriptor(framework);
			} catch {
				results.skipped.push({ framework, reason: "no arena.json yet" });
				continue;
			}
			const built = installable(descriptor, platform);
			if (!built) {
				results.skipped.push({
					framework,
					reason: descriptor.absent?.[platform] ??
						`${framework} does not run on ${platform}`,
				});
				continue;
			}
			try {
				const command = descriptor.targets[platform]?.build;
				if (!skipBuild && command) {
					await shell(
						command,
						`${ARENA_APPS_ROOT}/${framework}`,
						undefined,
						platform,
					);
				}
				await target.install(built.artifact);
			} catch (error) {
				results.skipped.push({
					framework,
					reason: `build or install failed: ${(error as Error).message}`,
				});
				continue;
			}
			ready.push({ framework, descriptor, app: built.app });
			results.frameworks[framework] = {
				version: descriptor.version,
				caveat: platform === "ios"
					? descriptor.targets.ios?.simulatorCaveat
					: undefined,
				scenarios: {},
			};
		}

		const samples = new Map<string, Map<string, Map<string, number[]>>>();
		const outcomes = new Map<
			string,
			Map<string, { failures: string[]; notes: Set<string> }>
		>();
		for (let round = 0; round <= runs; round += 1) {
			const warmUp = round === 0;
			const order = round % 2 === 1 ? ready : [...ready].reverse();
			for (const scenario of scenarios) {
				const launches = order.flatMap((entry) => [
					{ ...entry, lane: undefined },
					...(entry.descriptor.lanes?.[scenario] ?? []).map((lane) => ({
						...entry,
						lane,
					})),
				]);
				for (const { framework, descriptor, app, lane } of launches) {
					const unsupported = unsupportedReason(descriptor, scenario, platform);
					const outcome = outcomes.get(framework)?.get(scenario) ??
						{ failures: [], notes: new Set<string>() };
					if (!outcomes.has(framework)) outcomes.set(framework, new Map());
					outcomes.get(framework)!.set(scenario, outcome);
					if (unsupported) continue;
					log(
						`${
							warmUp ? "warm-up" : `round ${round}/${runs}`
						}: ${framework} ${scenario}${
							lane ? ` (${lane})` : ""
						} on ${platform}`,
					);
					let capture: Capture;
					try {
						capture = await target.run({
							app,
							scenario,
							lane,
							record: !warmUp && RECORDED.has(scenario),
							timeoutMs: TIMEOUT_MS[scenario],
						});
					} catch (error) {
						outcome.failures.push((error as Error).message);
						continue;
					}
					const failed = capture.reports.find((report: ArenaReport) =>
						!report.ok
					);
					if (capture.reports.length === 0 || failed) {
						outcome.failures.push(
							failed?.error ?? `no report within ${TIMEOUT_MS[scenario]} ms`,
						);
						continue;
					}
					for (const report of capture.reports) {
						for (const note of report.notes ?? []) {
							outcome.notes.add(note);
						}
					}
					if (warmUp) continue;
					let lanes: Map<string, Record<string, number>>;
					try {
						lanes = await measure(scenario, capture, target.info.refreshHz);
					} catch (error) {
						outcome.failures.push(
							`recording analysis failed: ${(error as Error).message}`,
						);
						continue;
					}
					for (const [lane, metrics] of lanes) {
						const perFramework = samples.get(framework) ??
							new Map<string, Map<string, number[]>>();
						samples.set(framework, perFramework);
						const perLane = perFramework.get(lane) ??
							new Map<string, number[]>();
						perFramework.set(lane, perLane);
						for (const [key, value] of Object.entries(metrics)) {
							if (Number.isFinite(value)) {
								perLane.set(key, [...(perLane.get(key) ?? []), value]);
							}
						}
					}
				}
			}
		}

		for (const { framework, descriptor } of ready) {
			const scenarioResults = results.frameworks[framework].scenarios;
			for (const scenario of scenarios) {
				const unsupported = unsupportedReason(descriptor, scenario, platform);
				if (unsupported) {
					scenarioResults[scenario] = {
						status: "unsupported",
						reason: unsupported,
						notes: [],
						metrics: {},
					};
					continue;
				}
				const outcome = outcomes.get(framework)?.get(scenario);
				const lanes = [...(samples.get(framework)?.entries() ?? [])]
					.filter(([lane]) =>
						lane === scenario || lane === `${scenario}.tuned`
					);
				if (lanes.length === 0) {
					scenarioResults[scenario] = {
						status: "failed",
						reason: outcome?.failures[0] ?? "no samples",
						notes: [...(outcome?.notes ?? [])],
						metrics: {},
					};
					continue;
				}
				for (const [lane, metrics] of lanes) {
					scenarioResults[lane] = {
						status: "ok",
						reason: outcome && outcome.failures.length > 0
							? `${outcome.failures.length} of ${runs + 1} launches failed: ${
								outcome.failures[0]
							}`
							: undefined,
						notes: [...(outcome?.notes ?? [])],
						metrics: Object.fromEntries(
							[...metrics].map(([key, values]) => [key, summarise(values)]),
						),
					};
				}
			}
		}

		const out = `${outDir}/arena-${platform}.json`;
		await Deno.writeTextFile(out, `${JSON.stringify(results, null, "\t")}\n`);
		log(`wrote ${out}`);
	}
}

if (import.meta.main) await main();
