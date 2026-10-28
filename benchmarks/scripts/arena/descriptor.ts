// The descriptor every arena app ships as `arena/apps/<framework>/arena.json`:
// how to sync its versions, build it per platform, find what the build made,
// and launch it. The harness knows nothing about any framework beyond this.
//
// Commands run with `bash -c` from the descriptor's directory. They may use
// `{versions}` (the absolute path of `arena/versions.json`) and, in tier
// builds, `{project}` (the absolute path of the generated tier project).
// Paths are relative to the descriptor's directory, or to the tier project
// for tier artifacts.

import { BENCHMARKS_ROOT } from "../common.ts";

export const ARENA_ROOT = `${BENCHMARKS_ROOT}/arena`;
export const ARENA_APPS_ROOT = `${ARENA_ROOT}/apps`;

export const PLATFORMS = [
	"ios",
	"android",
	"macos",
	"windows",
	"linux",
	"web",
] as const;
export type Platform = typeof PLATFORMS[number];

export const SCENARIOS = [
	"cold",
	"mount",
	"echo",
	"ticker",
	"autoscroll",
	"busy",
] as const;
export type Scenario = typeof SCENARIOS[number];

export interface IosTarget {
	/** Builds the Release app for the booted simulator (and, where it can, the device). */
	build: string;
	/** The simulator `.app` the build produced. */
	simulatorApp: string;
	/** The unsigned device `.app`, when the build makes one. */
	deviceApp?: string;
	bundleId: string;
	/**
	 * Why simulator numbers are not comparable for this framework, when they
	 * are not: Flutter, for one, cannot build Release for the simulator.
	 */
	simulatorCaveat?: string;
}

export interface AndroidTarget {
	/** Builds the arm64-v8a release APK. */
	build: string;
	apk: string;
	/** The launchable activity as `package/.Activity`. */
	component: string;
}

export interface DesktopTarget {
	build: string;
	/** The executable to launch with `--scenario <id>`. */
	executable: string;
	/** The bundle or directory the size benchmark measures. */
	artifact: string;
}

export interface WebTarget {
	build: string;
	/** The directory to serve; the app loads from its `index.html`. */
	dist: string;
}

export interface Targets {
	ios?: IosTarget;
	android?: AndroidTarget;
	macos?: DesktopTarget;
	windows?: DesktopTarget;
	linux?: DesktopTarget;
	web?: WebTarget;
}

export interface TierBuild {
	build: string;
	/** The artifact to measure, relative to the tier project. */
	artifact: string;
}

export interface Descriptor {
	framework: "leptoskit" | "react-native" | "lynx" | "flutter";
	/** The release being measured, as recorded in reports. */
	version: string;
	/** Syncs manifests and lockfiles from `{versions}`. Idempotent. */
	update: string;
	/** The scenario app, per platform it supports. */
	targets: Targets;
	/** Why a platform the framework supports in general has no arena target yet. */
	absent?: Partial<Record<Platform, string>>;
	/** Scenarios this framework cannot run as specified, with the reason. */
	unsupported?: Partial<Record<Scenario, string>>;
	/**
	 * Scenarios with a tuned lane that runs as its own launch, started with
	 * the `lane=tuned` argument. A tuned lane reported inside the idiomatic
	 * launch (a sync echo tier) needs no entry.
	 */
	lanes?: Partial<Record<Scenario, "tuned"[]>>;
	tiers: {
		/** The committed tier shell, copied fresh for every tier before emitting. */
		shell: string;
		/** The emitter module, relative to `scripts/arena/`. */
		emitter: string;
		platforms: Partial<Record<Platform, TierBuild>>;
	};
}

export function readDescriptor(framework: string): Descriptor {
	const path = `${ARENA_APPS_ROOT}/${framework}/arena.json`;
	return JSON.parse(Deno.readTextFileSync(path)) as Descriptor;
}

/** One `ARENA {json}` line, as every app prints it when a scenario finishes. */
export interface ArenaReport {
	v: 1;
	framework: string;
	frameworkVersion: string;
	platform: Platform;
	scenario: Scenario;
	lane: "idiomatic" | "tuned";
	ok: boolean;
	error?: string;
	notes?: string[];
	metrics: Record<string, number>;
	samples?: Record<string, number[]>;
}

const REPORT = /ARENA (\{.*)$/;
const PART = /ARENA-PART (\d+)\/(\d+) (.*)$/;

/**
 * Every report in a log, in order: whole `ARENA {json}` lines, and reports
 * split across consecutive `ARENA-PART i/n chunk` lines. Logs prefix lines
 * with their own metadata, so a marker may start anywhere in a line, and
 * some logs repeat a line (a console mirrored into the system log), so a
 * report identical to the previous one is dropped. A report that fails to
 * parse is an error, not a skip.
 */
export function parseReports(log: string): ArenaReport[] {
	const texts: string[] = [];
	let parts: string[] = [];
	for (const rawLine of log.split("\n")) {
		const line = rawLine.replace(/\r$/, "");
		const part = line.match(PART);
		if (part) {
			const [index, count] = [Number(part[1]), Number(part[2])];
			// A part for a new report restarts the assembly; a repeated part
			// (mirrored logs) is ignored.
			if (index === 1) parts = [];
			if (index === parts.length + 1) parts.push(part[3]);
			if (parts.length === count) {
				texts.push(parts.join(""));
				parts = [];
			}
			continue;
		}
		const whole = line.match(REPORT);
		if (whole) texts.push(whole[1].trimEnd());
	}
	const reports: ArenaReport[] = [];
	for (const [index, text] of texts.entries()) {
		if (index > 0 && text === texts[index - 1]) continue;
		reports.push(JSON.parse(text) as ArenaReport);
	}
	return reports;
}
