// Drivers for the places an arena app runs: an iOS simulator, an Android
// device or emulator, the macOS desktop, and a browser. Each one installs an
// app, launches it with a scenario while (optionally) recording the screen,
// and returns the app's report lines and the recording.
//
// Recordings start before the launch and stop half a second after the report
// arrives, so the done frame is always inside them. Everything a driver runs
// is a platform tool (simctl, adb, ffmpeg, Chromium), never framework tooling.

import { launchBrowser, type ScreencastFrame } from "../cdp.ts";
import { log } from "../common.ts";
import { serveStatic } from "../serve.ts";
import { type ArenaReport, parseReports, type Platform } from "./descriptor.ts";
import {
	beaconIn,
	type Box,
	decodePng,
	phaseIn,
	type PresentedFrame,
} from "./frames.ts";

export interface DeviceInfo {
	platform: Platform;
	/** Human-readable model and OS, as the results table prints it. */
	name: string;
	kind: "simulator" | "emulator" | "physical" | "desktop" | "browser";
	/** The display refresh rate frame statistics are computed against. */
	refreshHz: number;
}

export interface Launch {
	/** The installed app: a bundle id, `package/.Activity`, an executable, or a served directory. */
	app: string;
	scenario: string;
	/** A tuned lane launched on its own, passed as `lane=tuned`. */
	lane?: "tuned";
	record: boolean;
	timeoutMs: number;
}

export interface Capture {
	reports: ArenaReport[];
	/** The raw log text the reports came from, kept for failures. */
	log: string;
	/** The recording, when one was requested and the platform records video. */
	video?: string;
	/** Presented frames, when the platform reports them directly (the browser). */
	frames?: PresentedFrame[];
	/** Seconds into the recording at which the launch was issued. */
	launchedAt: number;
	/** Platform-reported launch time, when the platform reports one (Android's `am start -W`). */
	platformLaunchMs?: number;
}

export interface Target {
	info: DeviceInfo;
	install(artifact: string): Promise<void>;
	run(launch: Launch): Promise<Capture>;
}

// How long to keep recording after the report arrives: the done frame is set
// before the report is written, but a loaded compositor or a throttled
// screencast can deliver it later.
/** The launch arguments iOS and the desktops take: `--scenario <id> [--lane tuned]`. */
function argv(launch: Launch): string[] {
	return [
		"--scenario",
		launch.scenario,
		...(launch.lane ? ["--lane", launch.lane] : []),
	];
}

const SETTLE_MS = 1000;

function sleep(milliseconds: number): Promise<void> {
	return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function run(
	command: string,
	args: string[],
	allowFailure = false,
): Promise<string> {
	const result = await new Deno.Command(command, {
		args,
		stdout: "piped",
		stderr: "piped",
	}).output();
	const stdout = new TextDecoder().decode(result.stdout);
	if (!result.success && !allowFailure) {
		throw new Error(
			`${command} ${args.join(" ")} failed:\n${
				new TextDecoder().decode(result.stderr)
			}`,
		);
	}
	return stdout;
}

/** Collects a child process's output as it arrives. */
class OutputTail {
	text = "";
	readonly done: Promise<void>;

	constructor(...streams: ReadableStream<Uint8Array>[]) {
		this.done = Promise.all(streams.map(async (stream) => {
			const decoder = new TextDecoder();
			for await (const chunk of stream) {
				this.text += decoder.decode(chunk, { stream: true });
			}
		})).then(() => {});
	}
}

/** Polls `read` until it holds a complete report or the timeout passes. */
async function waitForReport(
	read: () => Promise<string> | string,
	timeoutMs: number,
): Promise<string> {
	const deadline = Date.now() + timeoutMs;
	let text = "";
	while (Date.now() < deadline) {
		text = await read();
		try {
			if (parseReports(text).length > 0) return text;
		} catch {
			// A half-written report; keep polling.
		}
		await sleep(250);
	}
	return text;
}

function scratchFile(suffix: string): string {
	return Deno.makeTempFileSync({ prefix: "arena-", suffix });
}

// ---------------------------------------------------------------------------
// iOS simulator

export function iosSimulator(udid: string): Target {
	const listing = JSON.parse(
		new TextDecoder().decode(
			new Deno.Command("xcrun", { args: ["simctl", "list", "devices", "-j"] })
				.outputSync().stdout,
		),
	) as { devices: Record<string, { udid: string; name: string }[]> };
	let name = udid;
	for (const [runtime, devices] of Object.entries(listing.devices)) {
		const device = devices.find((candidate) => candidate.udid === udid);
		if (device) {
			name = `${device.name}, ${
				runtime.replace(/.*SimRuntime\./, "").replaceAll("-", ".")
			} simulator`;
		}
	}
	return {
		// The simulator presents at 60 Hz regardless of the modelled device.
		info: { platform: "ios", name, kind: "simulator", refreshHz: 60 },
		async install(artifact) {
			await run("xcrun", ["simctl", "install", udid, artifact]);
		},
		async run(launch) {
			await run("xcrun", ["simctl", "terminate", udid, launch.app], true);
			const stream = new Deno.Command("xcrun", {
				args: [
					"simctl",
					"spawn",
					udid,
					"log",
					"stream",
					"--style",
					"compact",
					"--level",
					"debug",
					"--predicate",
					'eventMessage CONTAINS "ARENA"',
				],
				stdout: "piped",
				stderr: "null",
			}).spawn();
			const streamed = new OutputTail(stream.stdout);
			await sleep(1000);

			let recorder: Deno.ChildProcess | undefined;
			let recorderOutput: OutputTail | undefined;
			let recordingStarted = performance.now();
			const video = launch.record ? scratchFile(".mp4") : undefined;
			if (video) {
				recorder = new Deno.Command("xcrun", {
					args: [
						"simctl",
						"io",
						udid,
						"recordVideo",
						"--codec=h264",
						"--force",
						video,
					],
					stdout: "piped",
					stderr: "piped",
				}).spawn();
				recorderOutput = new OutputTail(recorder.stdout, recorder.stderr);
				const deadline = Date.now() + 10_000;
				while (
					!/Recording started/i.test(recorderOutput.text) &&
					Date.now() < deadline
				) await sleep(50);
				recordingStarted = performance.now();
				await sleep(1000);
			}

			const launchedAt = (performance.now() - recordingStarted) / 1000;
			const app = new Deno.Command("xcrun", {
				args: [
					"simctl",
					"launch",
					"--console-pty",
					"--terminate-running-process",
					udid,
					launch.app,
					...argv(launch),
				],
				stdout: "piped",
				stderr: "piped",
			}).spawn();
			const console = new OutputTail(app.stdout, app.stderr);
			const text = await waitForReport(
				() => `${console.text}\n${streamed.text}`,
				launch.timeoutMs,
			);
			await sleep(SETTLE_MS);

			if (recorder) {
				recorder.kill("SIGINT");
				await recorder.status;
			}
			await run("xcrun", ["simctl", "terminate", udid, launch.app], true);
			for (const child of [app, stream]) {
				try {
					child.kill("SIGTERM");
				} catch {
					// already exited
				}
				await child.status;
			}
			// The console and the system log usually both carry the report;
			// prefer the console, which never truncates.
			const reports = safeReports(console.text).length > 0
				? safeReports(console.text)
				: safeReports(streamed.text);
			return {
				reports,
				log: text,
				video,
				launchedAt: Math.max(0, launchedAt - 0.25),
			};
		},
	};
}

function safeReports(text: string): ArenaReport[] {
	try {
		return parseReports(text);
	} catch {
		return [];
	}
}

// ---------------------------------------------------------------------------
// Android

const ADB = `${
	Deno.env.get("ANDROID_HOME") ?? `${Deno.env.get("HOME")}/Library/Android/sdk`
}/platform-tools/adb`;

export async function android(serial: string): Promise<Target> {
	const adb = (...args: string[]) => run(ADB, ["-s", serial, ...args]);
	const property = async (name: string) =>
		(await adb("shell", "getprop", name)).trim();
	const model = await property("ro.product.model");
	const release = await property("ro.build.version.release");
	const emulator = (await property("ro.kernel.qemu")) === "1" ||
		serial.startsWith("emulator-");
	// SurfaceFlinger reports the active display mode's refresh period.
	const display = await adb("shell", "dumpsys", "SurfaceFlinger");
	const refresh = display.match(/refresh-rate\s*[:=]\s*([0-9.]+)/i) ??
		display.match(/([0-9.]+)\s*Hz/);
	const refreshHz = refresh ? Math.round(Number(refresh[1])) : 60;
	return {
		info: {
			platform: "android",
			name: `${model}, Android ${release}${emulator ? " emulator" : ""}`,
			kind: emulator ? "emulator" : "physical",
			refreshHz,
		},
		async install(artifact) {
			await adb("install", "-r", "-d", artifact);
		},
		async run(launch) {
			const [packageName] = launch.app.split("/");
			await adb("shell", "am", "force-stop", packageName);
			await adb("logcat", "-c");
			const remote = "/data/local/tmp/arena.mp4";
			let recorder: Deno.ChildProcess | undefined;
			let recordingStarted = performance.now();
			if (launch.record) {
				recorder = new Deno.Command(ADB, {
					args: [
						"-s",
						serial,
						"shell",
						"screenrecord",
						"--bit-rate",
						"16000000",
						remote,
					],
					stdout: "null",
					stderr: "null",
				}).spawn();
				recordingStarted = performance.now();
				await sleep(1500);
			}
			const launchedAt = (performance.now() - recordingStarted) / 1000;
			const extras = launch.lane ? ["--es", "lane", launch.lane] : [];
			const started = await adb(
				"shell",
				"am",
				"start",
				"-S",
				"-W",
				"-n",
				launch.app,
				"--es",
				"scenario",
				launch.scenario,
				...extras,
			);
			const total = started.match(/TotalTime:\s*(\d+)/);
			const text = await waitForReport(
				() => adb("logcat", "-d", "-v", "raw"),
				launch.timeoutMs,
			);
			await sleep(SETTLE_MS);

			let video: string | undefined;
			if (recorder) {
				await adb("shell", "pkill", "-INT", "screenrecord").catch(() => "");
				await recorder.status;
				// screenrecord finalises the file after the signal.
				await sleep(1000);
				video = scratchFile(".mp4");
				await adb("pull", remote, video);
			}
			await adb("shell", "am", "force-stop", packageName);
			return {
				reports: safeReports(text),
				log: text,
				video,
				launchedAt: Math.max(0, launchedAt - 0.25),
				platformLaunchMs: total ? Number(total[1]) : undefined,
			};
		},
	};
}

// ---------------------------------------------------------------------------
// Desktops: macOS, Windows, and Linux

/** The avfoundation index of the main display, for ffmpeg's screen capture on macOS. */
async function mainScreenIndex(): Promise<string> {
	const result = await new Deno.Command("ffmpeg", {
		args: [
			"-hide_banner",
			"-f",
			"avfoundation",
			"-list_devices",
			"true",
			"-i",
			"",
		],
		stderr: "piped",
	}).output();
	const listing = new TextDecoder().decode(result.stderr);
	const match = listing.match(/\[(\d+)\] Capture screen 0/);
	if (!match) throw new Error(`no screen to capture:\n${listing}`);
	return match[1];
}

/** ffmpeg's input arguments for capturing the whole screen on this OS. */
async function screenInput(
	platform: "macos" | "windows" | "linux",
): Promise<string[]> {
	switch (platform) {
		case "macos":
			return [
				"-f",
				"avfoundation",
				"-capture_cursor",
				"0",
				"-pixel_format",
				"bgr0",
				"-framerate",
				"60",
				"-i",
				`${await mainScreenIndex()}:none`,
			];
		case "windows":
			return [
				"-f",
				"gdigrab",
				"-draw_mouse",
				"0",
				"-framerate",
				"60",
				"-i",
				"desktop",
			];
		case "linux":
			return [
				"-f",
				"x11grab",
				"-draw_mouse",
				"0",
				"-framerate",
				"60",
				"-i",
				Deno.env.get("DISPLAY") ?? ":0",
			];
	}
}

function osName(platform: "macos" | "windows" | "linux"): string {
	if (platform === "macos") {
		const version = new TextDecoder().decode(
			new Deno.Command("sw_vers", { args: ["-productVersion"] }).outputSync()
				.stdout,
		).trim();
		return `macOS ${version}`;
	}
	return `${platform === "windows" ? "Windows" : "Linux"} ${Deno.osRelease()}`;
}

export function desktop(platform: "macos" | "windows" | "linux"): Target {
	return {
		info: { platform, name: osName(platform), kind: "desktop", refreshHz: 60 },
		async install() {
			// Desktop apps run in place.
		},
		async run(launch) {
			let recorder: Deno.ChildProcess | undefined;
			let recordingStarted = performance.now();
			const video = launch.record ? scratchFile(".mkv") : undefined;
			if (video) {
				// A lossless constant-rate capture; mpdecimate later keeps only
				// changed frames. 60 fps matches the display, so a frame the
				// app presents is never merged with the next one.
				recorder = new Deno.Command("ffmpeg", {
					args: [
						"-v",
						"error",
						"-y",
						...await screenInput(platform),
						"-c:v",
						"libx264",
						"-preset",
						"ultrafast",
						"-qp",
						"0",
						video,
					],
					stdin: "piped",
					stdout: "null",
					stderr: "piped",
				}).spawn();
				recordingStarted = performance.now();
				await sleep(1500);
			}
			const launchedAt = (performance.now() - recordingStarted) / 1000;
			const app = new Deno.Command(launch.app, {
				args: argv(launch),
				stdout: "piped",
				stderr: "piped",
			}).spawn();
			const output = new OutputTail(app.stdout, app.stderr);
			const text = await waitForReport(() => output.text, launch.timeoutMs);
			await sleep(SETTLE_MS);
			if (recorder) {
				// ffmpeg finishes the file cleanly on `q`.
				const writer = recorder.stdin.getWriter();
				await writer.write(new TextEncoder().encode("q"));
				await writer.close();
				await recorder.status;
			}
			try {
				app.kill("SIGTERM");
			} catch {
				// already exited
			}
			await app.status;
			return {
				reports: safeReports(text),
				log: text,
				video,
				launchedAt: Math.max(0, launchedAt - 0.25),
			};
		},
	};
}

// ---------------------------------------------------------------------------
// The browser

/**
 * Headless Chromium at the display's pace (vsync on, unlike the throughput
 * benchmark). The screencast delivers a frame each time the compositor
 * presents a changed one, so its frames are already the presented frames;
 * the beacon's phase is read from each frame's pixels directly.
 */
export function browser(): Target {
	return {
		info: {
			platform: "web",
			name: "headless Chromium",
			kind: "browser",
			refreshHz: 60,
		},
		async install() {
			// Served in place.
		},
		async run(launch) {
			const chromium = await launchBrowser({ paced: true });
			const server = serveStatic(launch.app);
			const page = await chromium.newPage();
			try {
				await page.setViewport(390, 844);
				const screencast = launch.record
					? await page.startScreencast()
					: undefined;
				const query = new URLSearchParams({
					scenario: launch.scenario,
					...(launch.lane ? { lane: launch.lane } : {}),
				});
				await page.navigate(`${server.url}/?${query}`);
				const text = await waitForReport(
					() => page.consoleText(),
					launch.timeoutMs,
				);
				await sleep(SETTLE_MS);
				const frames = screencast
					? screencastFrames(await screencast.stop())
					: undefined;
				return { reports: safeReports(text), log: text, frames, launchedAt: 0 };
			} finally {
				await page.close();
				await server.close();
				await chromium.close();
			}
		},
	};
}

/** The beacon in the latest frame that shows it finished, searching back from the end. */
function lastBeacon(captured: ScreencastFrame[]): Box {
	for (const frame of captured.slice(-30).reverse()) {
		try {
			return beaconIn(decodePng(frame.png), "the screencast");
		} catch {
			// Not finished in this frame; try the one before.
		}
	}
	throw new Error(
		"no done or failed beacon in the screencast's last 30 frames",
	);
}

/** Presented frames from screencast images, timed from the first one. */
function screencastFrames(captured: ScreencastFrame[]): PresentedFrame[] {
	if (captured.length === 0) {
		throw new Error("the screencast captured no frames");
	}
	const beacon = lastBeacon(captured);
	const origin = captured[0].timestamp;
	log(`read ${captured.length} screencast frames`);
	return captured.map((frame) => ({
		pts: frame.timestamp - origin,
		phase: phaseIn(decodePng(frame.png), beacon),
	}));
}
