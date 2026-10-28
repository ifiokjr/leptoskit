// Reads presented frames out of a screen recording, the arena's
// framework-neutral frame counter.

import { inflateSync } from "node:zlib";
//
// Every arena app shows a beacon, a square whose colour marks the scenario's
// phase (see arena/README.md). The recording is decimated so that only frames
// whose pixels changed survive, each with its presentation timestamp; the
// beacon's colour in each surviving frame then splits the recording into the
// windows a scenario measured. Variable-frame-rate recordings (the iOS
// simulator's and Android's own recorders) already hold one frame per change;
// constant-rate captures (desktop, the web) are reduced to the same shape by
// ffmpeg's `mpdecimate`.

export type BeaconPhase = "idle" | "running" | "done" | "failed" | "other";

/** One presented frame: when it reached the screen, and what the beacon showed. */
export interface PresentedFrame {
	/** Seconds from the start of the recording. */
	pts: number;
	phase: BeaconPhase;
}

/** A pixel rectangle in the recording's coordinates. */
export interface Box {
	x: number;
	y: number;
	width: number;
	height: number;
}

/** A phase window: the first running frame to the first done (or failed) frame after it. */
export interface PhaseWindow {
	start: number;
	end: number;
	outcome: "done" | "failed";
}

export interface FrameStats {
	window_ms: number;
	frames_count: number;
	presented_fps: number;
	/** Σ (interval − refresh period) over intervals longer than 1.5 periods. */
	hitch_ms: number;
	/** Hitch milliseconds per second of window: Apple's hitch time ratio. */
	hitch_ratio_ms_per_s: number;
	interval_p50_ms: number;
	interval_p95_ms: number;
	interval_p99_ms: number;
	longest_ms: number;
}

const BEACON_COLOURS: [
	Exclude<BeaconPhase, "other">,
	[number, number, number],
][] = [
	["idle", [0, 0, 0]],
	["running", [255, 0, 0]],
	["done", [0, 255, 0]],
	["failed", [0, 0, 255]],
];

// Video encoding moves pure colours by a few levels per channel (YUV 4:2:0
// rounding); anything further than this from every beacon colour is "other".
const COLOUR_TOLERANCE = 60;

/** The beacon phase nearest an RGB triple, or "other" outside the tolerance. */
export function classify(
	red: number,
	green: number,
	blue: number,
): BeaconPhase {
	let best: BeaconPhase = "other";
	let bestDistance = COLOUR_TOLERANCE;
	for (const [phase, [r, g, b]] of BEACON_COLOURS) {
		const distance = Math.max(
			Math.abs(red - r),
			Math.abs(green - g),
			Math.abs(blue - b),
		);
		if (distance <= bestDistance) {
			best = phase;
			bestDistance = distance;
		}
	}
	return best;
}

async function output(
	command: string,
	args: string[],
): Promise<{ stdout: Uint8Array; stderr: string }> {
	const result = await new Deno.Command(command, {
		args,
		stdout: "piped",
		stderr: "piped",
	}).output();
	const stderr = new TextDecoder().decode(result.stderr);
	if (!result.success) {
		throw new Error(
			`${command} ${args.join(" ")} failed:\n${stderr.slice(-2000)}`,
		);
	}
	return { stdout: result.stdout, stderr };
}

/** An 8-bit RGB image. */
export interface Pixmap {
	width: number;
	height: number;
	pixels: Uint8Array;
}

/** Parses a binary PPM (P6) image, as ffmpeg writes it. */
function parsePpm(bytes: Uint8Array): Pixmap {
	// Header: "P6" <ws> width <ws> height <ws> maxval <single ws> pixels.
	let offset = 0;
	const fields: string[] = [];
	while (fields.length < 4) {
		while (/\s/.test(String.fromCharCode(bytes[offset]))) offset += 1;
		let field = "";
		while (!/\s/.test(String.fromCharCode(bytes[offset]))) {
			field += String.fromCharCode(bytes[offset]);
			offset += 1;
		}
		fields.push(field);
	}
	offset += 1;
	if (fields[0] !== "P6" || fields[3] !== "255") {
		throw new Error(`unexpected PPM header: ${fields.join(" ")}`);
	}
	const width = Number(fields[1]);
	const height = Number(fields[2]);
	return {
		width,
		height,
		pixels: bytes.subarray(offset, offset + width * height * 3),
	};
}

/**
 * Finds the beacon in the recording's last frame, where every finished
 * scenario leaves it done (green) or failed (blue): the bounding box of the
 * largest connected run of beacon-coloured pixels.
 */
export async function locateBeacon(video: string): Promise<Box> {
	const scratch = await Deno.makeTempFile({ suffix: ".ppm" });
	try {
		// `-update 1` keeps overwriting one image, leaving the last frame.
		await output("ffmpeg", [
			"-v",
			"error",
			"-y",
			"-i",
			video,
			"-fps_mode",
			"passthrough",
			"-pix_fmt",
			"rgb24",
			"-update",
			"1",
			"-f",
			"image2",
			scratch,
		]);
		return beaconIn(parsePpm(await Deno.readFile(scratch)), video);
	} finally {
		await Deno.remove(scratch).catch(() => {});
	}
}

/** The beacon in a finished scenario's image: its done or failed square. */
export function beaconIn(image: Pixmap, source: string): Box {
	for (const phase of ["done", "failed"] as const) {
		const box = largestComponent(image, phase);
		if (box) return box;
	}
	throw new Error(`no done or failed beacon in the last frame of ${source}`);
}

/** The beacon's phase in one image, sampled over the middle of its box. */
export function phaseIn(image: Pixmap, beacon: Box): BeaconPhase {
	const [x0, y0] = [
		beacon.x + Math.floor(beacon.width / 4),
		beacon.y + Math.floor(beacon.height / 4),
	];
	const [width, height] = [
		Math.max(1, Math.floor(beacon.width / 2)),
		Math.max(1, Math.floor(beacon.height / 2)),
	];
	const totals = [0, 0, 0];
	for (let y = y0; y < y0 + height; y += 1) {
		for (let x = x0; x < x0 + width; x += 1) {
			const base = (y * image.width + x) * 3;
			for (let channel = 0; channel < 3; channel += 1) {
				totals[channel] += image.pixels[base + channel];
			}
		}
	}
	const count = width * height;
	return classify(totals[0] / count, totals[1] / count, totals[2] / count);
}

/**
 * Decodes an 8-bit, non-interlaced PNG (RGB or RGBA, as Chromium's
 * screencast writes) to RGB. Anything else is rejected rather than misread.
 */
export function decodePng(bytes: Uint8Array): Pixmap {
	const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
	let offset = 8;
	let width = 0;
	let height = 0;
	let channels = 0;
	const compressed: Uint8Array[] = [];
	while (offset < bytes.length) {
		const length = view.getUint32(offset);
		const type = String.fromCharCode(...bytes.subarray(offset + 4, offset + 8));
		const data = bytes.subarray(offset + 8, offset + 8 + length);
		if (type === "IHDR") {
			width = view.getUint32(offset + 8);
			height = view.getUint32(offset + 12);
			const [depth, colour, , , interlace] = data.subarray(8, 13);
			channels = colour === 2 ? 3 : colour === 6 ? 4 : 0;
			if (depth !== 8 || channels === 0 || interlace !== 0) {
				throw new Error(
					`unsupported PNG: depth ${depth}, colour type ${colour}, interlace ${interlace}`,
				);
			}
		} else if (type === "IDAT") {
			compressed.push(data);
		} else if (type === "IEND") {
			break;
		}
		offset += 12 + length;
	}
	const joined = new Uint8Array(
		compressed.reduce((sum, part) => sum + part.length, 0),
	);
	let at = 0;
	for (const part of compressed) {
		joined.set(part, at);
		at += part.length;
	}
	const raw = inflateSync(joined);
	const stride = width * channels;
	const current = new Uint8Array(stride);
	const previous = new Uint8Array(stride);
	const pixels = new Uint8Array(width * height * 3);
	for (let y = 0; y < height; y += 1) {
		const filter = raw[y * (stride + 1)];
		const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
		for (let index = 0; index < stride; index += 1) {
			const left = index >= channels ? current[index - channels] : 0;
			const up = previous[index];
			const upLeft = index >= channels ? previous[index - channels] : 0;
			let predictor = 0;
			if (filter === 1) predictor = left;
			else if (filter === 2) predictor = up;
			else if (filter === 3) predictor = (left + up) >> 1;
			else if (filter === 4) {
				const estimate = left + up - upLeft;
				const [toLeft, toUp, toUpLeft] = [
					Math.abs(estimate - left),
					Math.abs(estimate - up),
					Math.abs(estimate - upLeft),
				];
				predictor = toLeft <= toUp && toLeft <= toUpLeft
					? left
					: toUp <= toUpLeft
					? up
					: upLeft;
			}
			current[index] = (line[index] + predictor) & 0xff;
		}
		for (let x = 0; x < width; x += 1) {
			pixels.set(
				current.subarray(x * channels, x * channels + 3),
				(y * width + x) * 3,
			);
		}
		previous.set(current);
	}
	return { width, height, pixels };
}

// Beacons are at least 32 pt; at 1x that is 32 px, so anything smaller is a
// stray pixel of the same colour, not the beacon.
const MINIMUM_BEACON_PX = 16;

function largestComponent(image: Pixmap, phase: BeaconPhase): Box | null {
	const { width, height, pixels } = image;
	const mask = new Uint8Array(width * height);
	for (let index = 0; index < width * height; index += 1) {
		const base = index * 3;
		if (classify(pixels[base], pixels[base + 1], pixels[base + 2]) === phase) {
			mask[index] = 1;
		}
	}
	let best: Box | null = null;
	let bestArea = 0;
	const stack: number[] = [];
	for (let start = 0; start < mask.length; start += 1) {
		if (mask[start] !== 1) continue;
		mask[start] = 2;
		stack.push(start);
		let [minX, minY, maxX, maxY, area] = [width, height, 0, 0, 0];
		while (stack.length > 0) {
			const index = stack.pop()!;
			const x = index % width;
			const y = (index - x) / width;
			area += 1;
			minX = Math.min(minX, x);
			maxX = Math.max(maxX, x);
			minY = Math.min(minY, y);
			maxY = Math.max(maxY, y);
			for (const next of [index - 1, index + 1, index - width, index + width]) {
				if (next < 0 || next >= mask.length || mask[next] !== 1) continue;
				// Do not wrap from one row's end to the next row's start.
				if (Math.abs((next % width) - x) > 1) continue;
				mask[next] = 2;
				stack.push(next);
			}
		}
		const box = {
			x: minX,
			y: minY,
			width: maxX - minX + 1,
			height: maxY - minY + 1,
		};
		if (
			area > bestArea && box.width >= MINIMUM_BEACON_PX &&
			box.height >= MINIMUM_BEACON_PX
		) {
			best = box;
			bestArea = area;
		}
	}
	return best;
}

/**
 * Every presented frame of a recording, with the beacon's phase in it.
 *
 * `mpdecimate` drops frames identical to the last kept one (at its default
 * thresholds, which ignore encoder noise but keep a changed glyph), so a
 * frame survives only if the screen changed: one survivor per presented
 * frame, for every capture source.
 */
export async function presentedFrames(
	video: string,
	beacon: Box,
): Promise<PresentedFrame[]> {
	// Sample the middle half of the beacon, away from anti-aliased edges.
	const inset = {
		x: beacon.x + Math.floor(beacon.width / 4),
		y: beacon.y + Math.floor(beacon.height / 4),
		width: Math.max(2, Math.floor(beacon.width / 2)),
		height: Math.max(2, Math.floor(beacon.height / 2)),
	};
	const filter = [
		"mpdecimate",
		"showinfo",
		`crop=${inset.width}:${inset.height}:${inset.x}:${inset.y}`,
		"scale=1:1:flags=area",
	].join(",");
	const { stdout, stderr } = await output("ffmpeg", [
		"-v",
		"info",
		"-i",
		video,
		"-vf",
		filter,
		"-fps_mode",
		"passthrough",
		"-pix_fmt",
		"rgb24",
		"-f",
		"rawvideo",
		"pipe:1",
	]);
	const times = [...stderr.matchAll(/Parsed_showinfo.*?pts_time:\s*([0-9.]+)/g)]
		.map((match) => Number(match[1]));
	if (times.length * 3 !== stdout.length) {
		throw new Error(
			`ffmpeg reported ${times.length} frames but wrote ${
				stdout.length / 3
			} pixels for ${video}`,
		);
	}
	return times.map((pts, index) => ({
		pts,
		phase: classify(
			stdout[index * 3],
			stdout[index * 3 + 1],
			stdout[index * 3 + 2],
		),
	}));
}

/** Every running→done (or running→failed) window, in order. */
export function phaseWindows(frames: PresentedFrame[]): PhaseWindow[] {
	const windows: PhaseWindow[] = [];
	let start: number | null = null;
	for (const frame of frames) {
		if (frame.phase === "running" && start === null) start = frame.pts;
		if (
			(frame.phase === "done" || frame.phase === "failed") && start !== null
		) {
			windows.push({ start, end: frame.pts, outcome: frame.phase });
			start = null;
		}
	}
	return windows;
}

function percentile(sorted: number[], fraction: number): number {
	if (sorted.length === 0) return 0;
	const index = Math.min(
		sorted.length - 1,
		Math.ceil(fraction * sorted.length) - 1,
	);
	return sorted[Math.max(0, index)];
}

/** Frame cadence inside one window, for a display refreshing at `refreshHz`. */
export function frameStats(
	frames: PresentedFrame[],
	window: PhaseWindow,
	refreshHz: number,
): FrameStats {
	const period = 1000 / refreshHz;
	const times = frames.filter((frame) =>
		frame.pts >= window.start && frame.pts <= window.end
	)
		.map((frame) => frame.pts * 1000);
	const intervals = times.slice(1).map((time, index) => time - times[index]);
	const windowMs = (window.end - window.start) * 1000;
	const hitchMs = intervals.filter((interval) => interval > period * 1.5)
		.reduce((sum, interval) => sum + interval - period, 0);
	const sorted = [...intervals].sort((left, right) => left - right);
	return {
		window_ms: windowMs,
		frames_count: times.length,
		presented_fps: windowMs > 0 ? intervals.length / (windowMs / 1000) : 0,
		hitch_ms: hitchMs,
		hitch_ratio_ms_per_s: windowMs > 0 ? hitchMs / (windowMs / 1000) : 0,
		interval_p50_ms: percentile(sorted, 0.5),
		interval_p95_ms: percentile(sorted, 0.95),
		interval_p99_ms: percentile(sorted, 0.99),
		longest_ms: sorted.at(-1) ?? 0,
	};
}

/**
 * Launch to ready, for `cold`: from the first frame that changed after the
 * launch was issued (the system's own reaction, the same for every framework
 * on a platform) to the first frame showing the done beacon.
 */
export function launchToReady(
	frames: PresentedFrame[],
	launchedAt: number,
): number | null {
	const reaction = frames.find((frame) => frame.pts >= launchedAt);
	const ready = frames.find((frame) =>
		frame.phase === "done" && frame.pts >= launchedAt
	);
	if (!reaction || !ready) return null;
	return (ready.pts - reaction.pts) * 1000;
}
