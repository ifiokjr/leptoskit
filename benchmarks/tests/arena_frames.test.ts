import { assert, assertAlmostEquals, assertEquals } from "jsr:@std/assert@1";
import {
	classify,
	effectiveRefreshHz,
	frameStats,
	launchToReady,
	locateBeacon,
	phaseWindows,
	presentedFrames,
} from "../scripts/arena/frames.ts";

const hasFfmpeg =
	new Deno.Command("bash", { args: ["-c", "command -v ffmpeg"] }).outputSync()
		.success;

Deno.test("classify maps encoded beacon colours and rejects everything else", () => {
	assertEquals(classify(3, 2, 1), "idle");
	assertEquals(classify(250, 12, 8), "running");
	assertEquals(classify(10, 240, 6), "done");
	assertEquals(classify(4, 9, 247), "failed");
	assertEquals(classify(255, 255, 255), "other");
	assertEquals(classify(128, 128, 0), "other");
});

Deno.test("phase windows pair each running run with the next done or failed frame", () => {
	const frames = [
		{ pts: 0, phase: "idle" as const },
		{ pts: 1, phase: "running" as const },
		{ pts: 2, phase: "running" as const },
		{ pts: 3, phase: "done" as const },
		{ pts: 4, phase: "idle" as const },
		{ pts: 5, phase: "running" as const },
		{ pts: 6, phase: "failed" as const },
	];
	assertEquals(phaseWindows(frames), [
		{ start: 1, end: 3, outcome: "done" },
		{ start: 5, end: 6, outcome: "failed" },
	]);
	// A still screen, then the launch one second later.
	assertEquals(launchToReady(frames, true), 2000);
	// A recorder that writes two frames as it starts, then the still lead-in.
	const doubledStart = [
		{ pts: 0, phase: "idle" as const },
		{ pts: 0.02, phase: "idle" as const },
		{ pts: 1.1, phase: "other" as const },
		{ pts: 1.5, phase: "done" as const },
	];
	assertEquals(Math.round(launchToReady(doubledStart, true)!), 400);
	// A status bar changes during the lead-in; the launch follows the longest stillness.
	const strayChange = [
		{ pts: 0, phase: "idle" as const },
		{ pts: 0.31, phase: "idle" as const },
		{ pts: 0.33, phase: "idle" as const },
		{ pts: 1.38, phase: "other" as const },
		{ pts: 1.78, phase: "done" as const },
	];
	assertEquals(Math.round(launchToReady(strayChange, true)!), 400);
	// A screencast that starts at navigation, with a still splash before the app.
	const fromNavigation = [
		{ pts: 0, phase: "other" as const },
		{ pts: 0.9, phase: "other" as const },
		{ pts: 1.2, phase: "done" as const },
	];
	assertEquals(launchToReady(fromNavigation, false), 1200);
});

Deno.test("an idle frame closes a run the recorder never saw finish", () => {
	// A fast app marks a step running, then done for a frame or two; a
	// recorder that dropped that done frame must not pair the next step's
	// done with the previous step's running frame.
	const frames = [
		{ pts: 0, phase: "running" as const },
		{ pts: 0.2, phase: "idle" as const },
		{ pts: 0.21, phase: "running" as const },
		{ pts: 0.23, phase: "done" as const },
	];
	assertEquals(phaseWindows(frames), [{
		start: 0.21,
		end: 0.23,
		outcome: "done",
	}]);
});

const WIDTH = 120;
const HEIGHT = 200;
const PERIOD = 1 / 60;
// The capture runs at four times the display rate, like a constant-rate
// desktop capture: every presented frame arrives as four identical frames.
const CAPTURE_HZ = 240;

/** One synthetic screen, RGB: a beacon at (8, 8) and a bar that moves every frame. */
function screen(beacon: [number, number, number], bar: number): Uint8Array {
	const pixels = new Uint8Array(WIDTH * HEIGHT * 3).fill(255);
	for (let y = 0; y < HEIGHT; y += 1) {
		for (let x = 0; x < WIDTH; x += 1) {
			const base = (y * WIDTH + x) * 3;
			if (x >= 8 && x < 40 && y >= 8 && y < 40) pixels.set(beacon, base);
			else if (y >= 60 + (bar % 120) && y < 76 + (bar % 120)) {
				pixels.set([17, 17, 17], base);
			}
		}
	}
	return pixels;
}

Deno.test({
	name: "a recording's presented frames give its cadence and hitches",
	ignore: !hasFfmpeg,
	async fn() {
		const directory = await Deno.makeTempDir({ prefix: "arena-frames-" });
		try {
			// 10 idle display frames, 60 running frames at 60 Hz with one held
			// for three periods, then done frames.
			const shown: { image: Uint8Array; periods: number }[] = [];
			for (let index = 0; index < 10; index += 1) {
				shown.push({ image: screen([0, 0, 0], 0), periods: 1 });
			}
			for (let frame = 0; frame < 60; frame += 1) {
				shown.push({
					image: screen([255, 0, 0], frame + 1),
					periods: frame === 30 ? 3 : 1,
				});
			}
			for (let index = 0; index < 6; index += 1) {
				shown.push({ image: screen([0, 255, 0], 61), periods: 1 });
			}

			const video = `${directory}/recording.mkv`;
			const encoder = new Deno.Command("ffmpeg", {
				args: [
					"-v",
					"error",
					"-f",
					"rawvideo",
					"-pix_fmt",
					"rgb24",
					"-s",
					`${WIDTH}x${HEIGHT}`,
					"-framerate",
					String(CAPTURE_HZ),
					"-i",
					"pipe:0",
					"-c:v",
					"libx264",
					"-qp",
					"0",
					"-pix_fmt",
					"yuv444p",
					video,
				],
				stdin: "piped",
				stderr: "piped",
			}).spawn();
			const writer = encoder.stdin.getWriter();
			for (const { image, periods } of shown) {
				for (let copy = 0; copy < periods * CAPTURE_HZ / 60; copy += 1) {
					await writer.write(image);
				}
			}
			await writer.close();
			const encoded = await encoder.output();
			assert(encoded.success, new TextDecoder().decode(encoded.stderr));

			const beacon = await locateBeacon(video);
			assertEquals(beacon, { x: 8, y: 8, width: 32, height: 32 });

			const frames = await presentedFrames(video, beacon);
			assertEquals(frames.filter((frame) => frame.phase === "idle").length, 1);
			assertEquals(
				frames.filter((frame) => frame.phase === "running").length,
				60,
			);

			const [window] = phaseWindows(frames);
			const stats = frameStats(frames, window, 60);
			assertEquals(stats.frames_count, 61);
			// 60 intervals: 59 of one period and one of three, so one hitch of two periods.
			assertAlmostEquals(stats.window_ms, 62 * PERIOD * 1000, 2);
			assertAlmostEquals(stats.hitch_ms, 2 * PERIOD * 1000, 2);
			assertAlmostEquals(stats.longest_ms, 3 * PERIOD * 1000, 2);
			assertAlmostEquals(stats.presented_fps, 60 / 62 * 60, 1);
		} finally {
			await Deno.remove(directory, { recursive: true });
		}
	},
});

Deno.test("hitches are judged against the display mode the window ran in", () => {
	// A 120 Hz-capable display running an app at a steady 60 Hz.
	const frames = Array.from(
		{ length: 61 },
		(_, index) => ({ pts: index / 60, phase: "running" as const }),
	);
	const window = { start: 0, end: 1, outcome: "done" as const };
	assertEquals(effectiveRefreshHz(frames, window, [60, 90, 120]), 60);
	assertEquals(frameStats(frames, window, 60).hitch_ms, 0);
});
