// The arena app contract: scenarios, beacon phases, the report line, and the
// timing helpers every scenario shares.

import { useCallback, useLayoutEffect, useRef } from "react";
import { frameworkVersion, hostPlatform, writeLine } from "./host";

export const SCENARIOS = [
	"cold",
	"mount",
	"echo",
	"ticker",
	"autoscroll",
	"busy",
] as const;
export type Scenario = (typeof SCENARIOS)[number];

export function parseScenario(value: unknown): Scenario | null {
	return SCENARIOS.find((scenario) => scenario === value) ?? null;
}

export type Phase = "idle" | "running" | "done" | "failed";

export const PHASE_COLOR: Record<Phase, string> = {
	idle: "#000000",
	running: "#FF0000",
	done: "#00FF00",
	failed: "#0000FF",
};

export const TEXT_COLOR = "#111111";
export const BACKGROUND_COLOR = "#FFFFFF";

export interface Report {
	scenario: Scenario;
	lane: "idiomatic" | "tuned";
	ok: boolean;
	error?: string;
	notes?: string[];
	metrics: Record<string, number>;
	samples?: Record<string, number[]>;
}

/** System logs truncate long lines, so longer reports are split into parts of this many bytes. */
const MAX_LINE_BYTES = 800;

/** Writes the `ARENA {json}` report line, or `ARENA-PART i/n` lines for a long report. */
export function writeReport(report: Report): void {
	const notes = hostPlatform === "web"
		? [
			...(report.notes ?? []),
			`rendered by react-native-web ${frameworkVersion}`,
		]
		: report.notes;
	const json = JSON.stringify({
		v: 1,
		framework: "react-native",
		frameworkVersion,
		platform: hostPlatform,
		scenario: report.scenario,
		lane: report.lane,
		ok: report.ok,
		error: report.error,
		notes: notes !== undefined && notes.length > 0 ? notes : undefined,
		metrics: report.metrics,
		samples: report.samples,
	})
		// Escape anything outside ASCII so a chunk's length in characters is its length in bytes.
		.replace(
			/[\u0080-\uffff]/g,
			(char) => `\\u${char.charCodeAt(0).toString(16).padStart(4, "0")}`,
		);

	if (json.length <= MAX_LINE_BYTES) {
		writeLine(`ARENA ${json}`);
		return;
	}
	const count = Math.ceil(json.length / MAX_LINE_BYTES);
	for (let index = 0; index < count; index += 1) {
		const chunk = json.slice(
			index * MAX_LINE_BYTES,
			(index + 1) * MAX_LINE_BYTES,
		);
		writeLine(`ARENA-PART ${index + 1}/${count} ${chunk}`);
	}
}

export function failureReport(scenario: Scenario, error: unknown): Report {
	const message = error instanceof Error ? error.message : String(error);
	return {
		scenario,
		lane: "idiomatic",
		ok: false,
		error: message,
		metrics: {},
	};
}

export function median(values: readonly number[]): number {
	return percentile(values, 50);
}

/** The nearest-rank percentile of `values`. */
export function percentile(values: readonly number[], rank: number): number {
	if (values.length === 0) {
		return Number.NaN;
	}
	const sorted = [...values].sort((a, b) => a - b);
	const index = Math.min(
		sorted.length - 1,
		Math.max(0, Math.ceil((rank / 100) * sorted.length) - 1),
	);
	return sorted[index];
}

/** Rounds to three decimals so reports stay short without losing microsecond precision in milliseconds. */
export function round3(value: number): number {
	return Math.round(value * 1000) / 1000;
}

export function nextFrame(): Promise<number> {
	return new Promise((resolve) => requestAnimationFrame(resolve));
}

/**
 * Commits a state change and resolves with `performance.now()` read inside the
 * first `requestAnimationFrame` callback scheduled from the `useLayoutEffect`
 * that follows the commit: the earliest after-frame hook React Native offers. Layout effects run once the new
 * Fabric shadow tree is committed, and a state update made in one (a failed
 * check, say) is applied before the frame is painted.
 *
 * The component calling this must re-render for the update, since the hook
 * watches its own commits.
 */
export function useCommitFrame(
	onCommit?: () => void,
): (update: () => void) => Promise<number> {
	const pending = useRef<((now: number) => void) | null>(null);
	const onCommitRef = useRef(onCommit);
	onCommitRef.current = onCommit;

	useLayoutEffect(() => {
		const resolve = pending.current;
		if (resolve === null) {
			return;
		}
		pending.current = null;
		onCommitRef.current?.();
		// The callback's own timestamp is the frame's start, which can precede the
		// commit; the time the hook actually runs is what the step costs.
		requestAnimationFrame(() => resolve(performance.now()));
	});

	return useCallback(
		(update: () => void) =>
			new Promise<number>((resolve) => {
				pending.current = resolve;
				update();
			}),
		[],
	);
}
