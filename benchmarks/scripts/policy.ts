// Regression classification against the committed thresholds in
// `benchmarks/policy.json`. A metric regresses when it is worse than base by
// BOTH the percent and the absolute threshold — percent alone chases noise on
// sub-millisecond scenarios, absolute alone ignores real regressions on slow
// ones.
import type { Metric, MetricClass } from "./common.ts";

export interface PolicyThreshold {
	percent: number;
	absolute: number;
}

export interface PolicyClass {
	warn: PolicyThreshold;
	fail: PolicyThreshold;
}

export type Policy = Record<MetricClass, PolicyClass>;

export type Status = "improved" | "ok" | "warn" | "fail" | "missing";

/**
 * Which way head moved the metric. Independent of the sign of the change:
 * a duration cut and a throughput gain are both `better`.
 */
export type Direction = "better" | "worse" | "unchanged";

export interface Comparison {
	id: string;
	label: string;
	unit: Metric["unit"];
	base: number | null;
	head: number | null;
	/** Signed change in the metric's own unit: `head - base`. */
	delta: number | null;
	/** Percent change from base; null when base is zero. */
	percent: number | null;
	/** Which way head moved the metric; null when either side is missing. */
	direction: Direction | null;
	status: Status;
}

export async function loadPolicy(path: string): Promise<Policy> {
	return JSON.parse(await Deno.readTextFile(path)) as Policy;
}

/** Whether higher values are better for this metric class/unit pairing. */
function higherIsBetter(
	metricClass: MetricClass,
	unit: Metric["unit"],
): boolean {
	if (metricClass === "throughput") return true;
	return unit === "per-sec";
}

export function classify(
	id: string,
	label: string,
	unit: Metric["unit"],
	metricClass: MetricClass,
	base: number | undefined,
	head: number | undefined,
	policy: Policy,
): Comparison {
	if (base === undefined || head === undefined) {
		return {
			id,
			label,
			unit,
			base: base ?? null,
			head: head ?? null,
			delta: null,
			percent: null,
			direction: null,
			status: "missing",
		};
	}
	const worse = higherIsBetter(metricClass, unit) ? head < base : head > base;
	const delta = head - base;
	const percent = base === 0 ? null : (delta / base) * 100;
	const direction: Direction = delta === 0
		? "unchanged"
		: worse
		? "worse"
		: "better";
	const thresholds = policy[metricClass];
	// The magnitude of the change, expressed as worsening or improving.
	const magnitude = Math.abs(percent ?? 0);
	const crossed = (threshold: PolicyThreshold) =>
		percent === null
			? Math.abs(delta) >= threshold.absolute
			: magnitude >= threshold.percent && Math.abs(delta) >= threshold.absolute;

	const measured = { id, label, unit, base, head, delta, percent, direction };
	if (!worse) {
		return {
			...measured,
			status: crossed(thresholds.warn) ? "improved" : "ok",
		};
	}
	if (crossed(thresholds.fail)) return { ...measured, status: "fail" };
	if (crossed(thresholds.warn)) return { ...measured, status: "warn" };
	return { ...measured, status: "ok" };
}

/** The emoji marker for a comparison, used in tables and comments. */
export function statusEmoji(status: Status): string {
	switch (status) {
		case "improved":
			return "🟢";
		case "ok":
			return "➡️";
		case "warn":
			return "🟡";
		case "fail":
			return "🔴";
		case "missing":
			return "⚪";
	}
}

/**
 * The emoji marker for the direction of a change: green when head moved the
 * metric the good way, orange when it moved it the bad way — the sign of the
 * percentage alone does not say which is which.
 */
export function directionEmoji(direction: Direction): string {
	switch (direction) {
		case "better":
			return "🟢";
		case "worse":
			return "🟠";
		case "unchanged":
			return "➡️";
	}
}
