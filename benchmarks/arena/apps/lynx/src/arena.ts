// The arena app contract: scenario ids, beacon phases, the clock, and the
// `ARENA` report line. See `benchmarks/arena/README.md`.

import type { PerformanceEntry, PipelineEntry } from "@lynx-js/types";

export const SCENARIOS = [
	"cold",
	"mount",
	"echo",
	"ticker",
	"autoscroll",
	"busy",
] as const;
export type ScenarioId = typeof SCENARIOS[number];

export type Phase = "idle" | "running" | "done" | "failed";

export const PHASE_COLOURS: Record<Phase, string> = {
	idle: "#000000",
	running: "#FF0000",
	done: "#00FF00",
	failed: "#0000FF",
};

export type Lane = "idiomatic" | "tuned";

export interface ArenaReport {
	v: 1;
	framework: "lynx";
	frameworkVersion: string;
	platform: string;
	scenario: ScenarioId;
	lane: Lane;
	ok: boolean;
	error?: string;
	notes?: string[];
	metrics: Record<string, number>;
	samples?: Record<string, number[]>;
}

/** What a scenario hands back when it finishes; the app fills in the rest. */
export interface ScenarioResult {
	ok: boolean;
	error?: string;
	notes?: string[];
	metrics: Record<string, number>;
	samples?: Record<string, number[]>;
}

export function isScenario(value: string | undefined): value is ScenarioId {
	return SCENARIOS.some((scenario) => scenario === value);
}

export interface LaunchInfo {
	scenario: ScenarioId | undefined;
	lane: Lane;
	platform: string;
	frameworkVersion: string;
	refreshRate: number;
	processStartUnixMs: number | undefined;
}

/** Reads the launch data the host passed as global props. Safe on both threads. */
export function readLaunch(): LaunchInfo {
	const props = lynx.__globalProps;
	return {
		scenario: isScenario(props.scenario) ? props.scenario : undefined,
		lane: props.lane === "tuned" ? "tuned" : "idiomatic",
		platform: props.platform ?? "unknown",
		frameworkVersion: props.frameworkVersion ?? "unknown",
		refreshRate: props.refreshRate !== undefined && props.refreshRate > 0
			? props.refreshRate
			: 60,
		processStartUnixMs: props.processStartUnixMs,
	};
}

interface HighResolutionClock {
	now(): number;
}

function isClock(candidate: unknown): candidate is HighResolutionClock {
	return typeof candidate === "object" && candidate !== null &&
		typeof Reflect.get(candidate, "now") === "function";
}

/** A global function returning a number, looked up without assuming it exists on this thread. */
function globalFunction(name: string): (() => number) | undefined {
	const candidate: unknown = Reflect.get(globalThis, name);
	if (typeof candidate !== "function") return undefined;
	return () => {
		const value: unknown = Reflect.apply(candidate, undefined, []);
		return typeof value === "number" ? value : Number.NaN;
	};
}

/** The iOS host's `ArenaClock.nowUs()`: a monotonic microsecond clock, read synchronously. */
function nativeClock(): (() => number) | undefined {
	// `NativeModules` is in scope on the background thread only; `typeof`
	// keeps this safe where the main thread evaluates the module too.
	if (typeof NativeModules === "undefined") return undefined;
	const module: unknown = Reflect.get(NativeModules, "ArenaClock");
	if (typeof module !== "object" || module === null) return undefined;
	const nowUs: unknown = Reflect.get(module, "nowUs");
	if (typeof nowUs !== "function") return undefined;
	return () => {
		const value: unknown = Reflect.apply(nowUs, module, []);
		return typeof value === "number" ? value : Number.NaN;
	};
}

const performanceGlobal: unknown = Reflect.get(globalThis, "performance");
const performanceClock: HighResolutionClock | undefined =
	isClock(performanceGlobal) ? performanceGlobal : undefined;
/**
 * The native Lynx background runtimes differ: PrimJS (Android) has no
 * `performance.now()` but exposes `__date_clock()`, the wall clock in
 * microseconds since the Unix epoch behind its `Date`; JavaScriptCore (iOS)
 * has neither, so the iOS host provides `ArenaClock`. The web has
 * `performance.now()`.
 */
const dateClock = globalFunction("__date_clock");
const hostClock = performanceClock === undefined && dateClock === undefined
	? nativeClock()
	: undefined;

/** Which clock `now()` reads, for the report notes. */
export const CLOCK_SOURCE:
	| "performance.now"
	| "__date_clock"
	| "ArenaClock.nowUs"
	| "Date.now" = performanceClock !== undefined
		? "performance.now"
		: dateClock !== undefined
		? "__date_clock"
		: hostClock !== undefined
		? "ArenaClock.nowUs"
		: "Date.now";

/** Whether `now()` has sub-millisecond resolution. */
export const HAS_HIGH_RESOLUTION_CLOCK = CLOCK_SOURCE !== "Date.now";

/** Whether reading `now()` is itself a native module call, whose cost timings must subtract. */
export const CLOCK_IS_NATIVE_CALL = CLOCK_SOURCE === "ArenaClock.nowUs";

/** Milliseconds on the background thread, with sub-millisecond resolution where the runtime has it. */
export function now(): number {
	if (performanceClock !== undefined) return performanceClock.now();
	if (dateClock !== undefined) return dateClock() / 1000;
	if (hostClock !== undefined) return hostClock() / 1000;
	return Date.now();
}

/** Which clock `unixNow()` reads, for the report notes. */
export const UNIX_CLOCK_SOURCE: "__date_clock" | "Date.now" =
	dateClock !== undefined ? "__date_clock" : "Date.now";

/** Unix time in milliseconds, comparable with Lynx's `PerformanceEntry` timestamps. */
export function unixNow(): number {
	return dateClock !== undefined ? dateClock() / 1000 : Date.now();
}

/** Resolves on the next frame callback with its timestamp in milliseconds. */
export function nextFrame(): Promise<number> {
	return new Promise((resolve) => {
		requestAnimationFrame((timestamp) => resolve(timestamp ?? now()));
	});
}

export function sleep(ms: number): Promise<void> {
	return new Promise((resolve) => {
		setTimeout(() => resolve(), ms);
	});
}

/** Logs that truncate long lines (os_log at about 1 KB) get the report in parts. */
const REPORT_CHUNK_BYTES = 800;

function writeLine(line: string): void {
	const logger = NativeModules.ArenaLog;
	if (logger !== undefined) {
		logger.log(line);
	} else {
		console.log(line);
	}
}

/**
 * Writes one report: a single `ARENA {json}` line, or, past 800 bytes,
 * consecutive `ARENA-PART i/n <chunk>` lines. The JSON is ASCII (every
 * string in it is), so characters and bytes coincide.
 */
export function emitReport(report: ArenaReport): void {
	const json = JSON.stringify(report);
	if (json.length <= REPORT_CHUNK_BYTES) {
		writeLine(`ARENA ${json}`);
		return;
	}
	const count = Math.ceil(json.length / REPORT_CHUNK_BYTES);
	for (let index = 0; index < count; index += 1) {
		const chunk = json.slice(
			index * REPORT_CHUNK_BYTES,
			(index + 1) * REPORT_CHUNK_BYTES,
		);
		writeLine(`ARENA-PART ${index + 1}/${count} ${chunk}`);
	}
}

export function percentile(sorted: number[], fraction: number): number {
	if (sorted.length === 0) return Number.NaN;
	const index = Math.min(
		sorted.length - 1,
		Math.max(0, Math.ceil(fraction * sorted.length) - 1),
	);
	return sorted[index];
}

export function median(values: number[]): number {
	return percentile([...values].sort((a, b) => a - b), 0.5);
}

/** Rounds to three decimals so reports stay compact without losing microseconds. */
export function round3(value: number): number {
	return Math.round(value * 1000) / 1000;
}

/**
 * Waits for the rendering pipeline flagged with `identifier` through
 * `__lynx_timing_flag` to finish and reach the screen, and resolves with its
 * `PipelineEntry`. Lynx raises the entry from the engine after the UI
 * operations of that pipeline were applied and painted.
 */
export class PipelineWatcher {
	private readonly waiting = new Map<string, (entry: PipelineEntry) => void>();
	private readonly arrived = new Map<string, PipelineEntry>();
	private readonly listeners: ((entry: PerformanceEntry) => void)[] = [];
	/** The last few entries seen, named in timeout errors. */
	private readonly recent: string[] = [];

	constructor() {
		const observer = lynx.performance.createObserver(
			(entry: PerformanceEntry) => {
				for (const listener of this.listeners) listener(entry);
				const identifier: unknown = Reflect.get(entry, "identifier");
				this.recent.push(
					`${entry.entryType}.${entry.name}:${String(identifier)}`,
				);
				if (this.recent.length > 8) this.recent.shift();
				if (entry.entryType !== "pipeline") return;
				const pipeline = entry as PipelineEntry;
				if (pipeline.identifier === "") return;
				const resolve = this.waiting.get(pipeline.identifier);
				if (resolve !== undefined) {
					this.waiting.delete(pipeline.identifier);
					resolve(pipeline);
				} else {
					this.arrived.set(pipeline.identifier, pipeline);
				}
			},
		);
		observer.observe(["pipeline", "metric", "init"]);
	}

	/** Every entry, for scenarios that read load or metric entries. */
	listen(listener: (entry: PerformanceEntry) => void): void {
		this.listeners.push(listener);
	}

	waitFor(identifier: string, timeoutMs: number): Promise<PipelineEntry> {
		const early = this.arrived.get(identifier);
		if (early !== undefined) {
			this.arrived.delete(identifier);
			return Promise.resolve(early);
		}
		return new Promise((resolve, reject) => {
			const timer = setTimeout(() => {
				this.waiting.delete(identifier);
				reject(
					new Error(
						`no pipeline entry for ${identifier} within ${timeoutMs} ms (recent entries: ${
							this.recent.join(", ") || "none"
						})`,
					),
				);
			}, timeoutMs);
			this.waiting.set(identifier, (entry) => {
				clearTimeout(timer);
				resolve(entry);
			});
		});
	}
}
