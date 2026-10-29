import {
	runOnBackground,
	runOnMainThread,
	useEffect,
	useMainThreadRef,
} from "@lynx-js/react";
import type { MainThread } from "@lynx-js/types";

import type { ScenarioProps } from "../App.js";
import { PHASE_COLOURS, readLaunch, round3 } from "../arena.js";
import { readTileZero, TICKER_DURATION_MS, TILES } from "./Ticker.js";

interface TunedResult {
	frames: number;
	skipped: number;
	shown: string;
}

/**
 * Ticker wall, tuned lane: the frame loop runs as a main-thread script. Each
 * main-thread `requestAnimationFrame` callback writes every tile's `text`
 * attribute directly on its element, so no update crosses threads. The
 * beacon is set from the same callbacks, in the same frames.
 */
export function TickerTuned(props: ScenarioProps) {
	const { setPhase, finish } = props;
	const gridRef = useMainThreadRef<MainThread.Element | null>(null);

	useEffect(() => {
		const vsyncMs = 1000 / readLaunch().refreshRate;
		const report = (result: TunedResult) => {
			if (result.shown !== String(result.frames)) {
				finish({
					ok: false,
					error: `tile 0 shows ${
						JSON.stringify(result.shown)
					}, expected ${result.frames}`,
					metrics: {},
				});
				return;
			}
			finish({
				ok: true,
				metrics: {
					frames_count: result.frames,
					skipped_count: result.skipped,
					final_frame: result.frames,
					vsync_ms: round3(vsyncMs),
				},
				notes: [
					"tuned lane: a main-thread script requestAnimationFrame loop sets each tile's text attribute on its element",
				],
			});
		};
		const syncPhase = (phase: "running" | "done") => setPhase(phase);

		void runOnMainThread(
			(durationMs: number, vsync: number, running: string, done: string) => {
				"main thread";
				const grid = gridRef.current;
				const beacon = lynx.querySelector(".beacon");
				if (grid === null || beacon === null) return;
				const tiles = grid.querySelectorAll(".tile");
				let frames = 0;
				let skipped = 0;
				let start = 0;
				let previous = 0;
				const tick = () => {
					const at = Date.now();
					if (frames === 0) {
						start = at;
					} else if (at - previous > 1.5 * vsync) {
						skipped += 1;
					}
					previous = at;
					frames += 1;
					for (let index = 0; index < tiles.length; index += 1) {
						tiles[index].setAttribute("text", String(frames + index));
					}
					const finished = at - start >= durationMs;
					if (frames === 1) {
						beacon.setStyleProperty("background-color", running);
						runOnBackground(syncPhase)("running");
					}
					if (finished) {
						beacon.setStyleProperty("background-color", done);
						requestAnimationFrame(() => {
							runOnBackground(report)({
								frames,
								skipped,
								shown: readTileZero(grid),
							});
						});
						return;
					}
					requestAnimationFrame(tick);
				};
				requestAnimationFrame(tick);
			},
		)(TICKER_DURATION_MS, vsyncMs, PHASE_COLOURS.running, PHASE_COLOURS.done);
	}, []);

	return (
		<view className="grid" main-thread:ref={gridRef}>
			{TILES.map((index) => (
				<text key={index} className="tile">{String(index)}</text>
			))}
		</view>
	);
}
