import { useEffect, useMemo, useRef } from "@lynx-js/react";
import type { LoadBundleEntry, PerformanceEntry } from "@lynx-js/types";

import type { ScenarioProps } from "../App.js";
import { nextFrame, readLaunch, round3, sleep } from "../arena.js";

const ROWS = Array.from({ length: 20 }, (_, index) => `Row ${index + 1}`);

/** How long to wait for the engine's load entry before reporting without it. */
const LOAD_ENTRY_TIMEOUT_MS = 3000;

/** `@lynx-js/web-core` has no performance observer; the native engines do. */
export function hasPerformanceObserver(): boolean {
	return readLaunch().platform !== "web";
}

function isLoadBundle(entry: PerformanceEntry): entry is LoadBundleEntry {
	return entry.entryType === "pipeline" && entry.name === "loadBundle";
}

/**
 * The ready screen: the header and 20 rows. Lynx renders the first screen on
 * the main thread straight from the bundle (instant first-frame rendering),
 * so the beacon is already `done` in the first frame that shows the rows.
 * `first_frame_ms` is the load pipeline's paint end relative to process start.
 */
export function Cold(props: ScenarioProps) {
	const { finish } = props;
	const loadEntry = useRef<LoadBundleEntry | undefined>(undefined);
	const waiters = useRef<(() => void)[]>([]);

	useMemo(() => {
		"background only";
		if (!hasPerformanceObserver()) return;
		// Registered during the first background render so the load entry,
		// which the engine raises once, is not missed.
		const observer = lynx.performance.createObserver(
			(entry: PerformanceEntry) => {
				if (!isLoadBundle(entry)) return;
				loadEntry.current = entry;
				for (const wake of waiters.current) wake();
			},
		);
		observer.observe(["pipeline.loadBundle"]);
	}, []);

	useEffect(() => {
		const launch = readLaunch();
		const notes: string[] = [];
		const metrics: Record<string, number> = {};
		const loaded = loadEntry.current !== undefined
			? Promise.resolve()
			: new Promise<void>((resolve) => waiters.current.push(resolve));
		const waitForLoad = hasPerformanceObserver()
			? Promise.race([loaded, sleep(LOAD_ENTRY_TIMEOUT_MS)])
			: nextFrame();
		void waitForLoad.then(() => {
			const entry = loadEntry.current;
			if (!hasPerformanceObserver()) {
				notes.push(
					"the web engine raises no pipeline entries: first_frame_ms is not reported",
				);
			} else if (entry === undefined) {
				notes.push(
					"no loadBundle pipeline entry: first_frame_ms is not reported",
				);
			} else if (launch.processStartUnixMs === undefined) {
				notes.push(
					"the host does not know its process start: first_frame_ms is not reported",
				);
			} else {
				metrics.first_frame_ms = round3(
					entry.paintEnd - launch.processStartUnixMs,
				);
				metrics.load_bundle_ms = round3(entry.paintEnd - entry.loadBundleStart);
				notes.push(
					"first_frame_ms is the loadBundle pipeline's paintEnd minus the process start time",
				);
			}
			finish({ ok: true, metrics, notes });
		});
	}, []);

	return (
		<view className="fill">
			{ROWS.map((row) => <text key={row} className="row">{row}</text>)}
		</view>
	);
}
