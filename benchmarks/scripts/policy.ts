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

export interface Comparison {
	id: string;
	label: string;
	unit: Metric["unit"];
	base: number | null;
	head: number | null;
	/** Signed change in the metric's own unit; positive = head is worse. */
	delta: number | null;
	/** Percent change; positive = head is worse. */
	percent: number | null;
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
			status: "missing",
		};
	}
	const worse = higherIsBetter(metricClass, unit) ? head < base : head > base;
	const delta = head - base;
	const percent = base === 0 ? null : (delta / base) * 100;
	const thresholds = policy[metricClass];
	// The magnitude of the change, expressed as worsening or improving.
	const magnitude = Math.abs(percent ?? 0);
	const crossed = (threshold: PolicyThreshold) =>
		percent === null
			? Math.abs(delta) >= threshold.absolute
			: magnitude >= threshold.percent && Math.abs(delta) >= threshold.absolute;

	if (!worse) {
		return {
			id,
			label,
			unit,
			base,
			head,
			delta,
			percent,
			status: crossed(thresholds.warn) ? "improved" : "ok",
		};
	}
	if (crossed(thresholds.fail)) {
		return { id, label, unit, base, head, delta, percent, status: "fail" };
	}
	if (crossed(thresholds.warn)) {
		return { id, label, unit, base, head, delta, percent, status: "warn" };
	}
	return { id, label, unit, base, head, delta, percent, status: "ok" };
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
