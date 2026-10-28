import {
	runOnMainThread,
	useEffect,
	useMainThreadRef,
	useState,
} from "@lynx-js/react";
import type { MainThread } from "@lynx-js/types";

import type { ScenarioProps } from "../App.js";
import { nextFrame, now, readLaunch, round3 } from "../arena.js";
import { textOf } from "../elements.js";

export const COLUMNS = 15;
export const ROWS = 20;
export const TILES = Array.from(
	{ length: COLUMNS * ROWS },
	(_, index) => index,
);
export const TICKER_DURATION_MS = 5000;

/** Reads what tile 0 shows, on the main thread. */
export function readTileZero(grid: MainThread.Element | null): string {
	"main thread";
	if (grid === null) return "";
	return textOf(grid.querySelector(".tile"));
}

/**
 * Ticker wall, idiomatic lane: a background-thread `requestAnimationFrame`
 * loop sets one piece of React state per frame, and the 300 tiles render
 * `frame + i` from it. Every update travels background thread → diff →
 * main thread → native views.
 */
export function Ticker(props: ScenarioProps) {
	const { setPhase, finish } = props;
	const [frame, setFrame] = useState(0);
	const gridRef = useMainThreadRef<MainThread.Element | null>(null);

	useEffect(() => {
		const vsyncMs = 1000 / readLaunch().refreshRate;

		async function run(): Promise<void> {
			await nextFrame();
			let frames = 0;
			let skipped = 0;
			let previous = now();
			const start = previous;
			let finished = false;
			while (!finished) {
				await nextFrame();
				const at = now();
				if (at - previous > 1.5 * vsyncMs) skipped += 1;
				previous = at;
				frames += 1;
				finished = at - start >= TICKER_DURATION_MS;
				setFrame(frames);
				if (frames === 1) setPhase("running");
				if (finished) setPhase("done");
			}
			await nextFrame();
			await nextFrame();
			const shown = await runOnMainThread((): string => {
				"main thread";
				return readTileZero(gridRef.current);
			})();
			if (shown !== String(frames)) {
				throw new Error(
					`tile 0 shows ${JSON.stringify(shown)}, expected ${frames}`,
				);
			}
			finish({
				ok: true,
				metrics: {
					frames_count: frames,
					skipped_count: skipped,
					final_frame: frames,
					vsync_ms: round3(vsyncMs),
				},
				notes: [
					"frame callbacks are background-thread requestAnimationFrame; tiles update through useState",
				],
			});
		}

		run().catch((error: unknown) => {
			finish({
				ok: false,
				error: error instanceof Error ? error.message : String(error),
				metrics: {},
			});
		});
	}, []);

	return (
		<view className="grid" main-thread:ref={gridRef}>
			{TILES.map((index) => (
				<text key={index} className="tile">{String(frame + index)}</text>
			))}
		</view>
	);
}
