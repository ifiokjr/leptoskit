import { memo, type Ref, useEffect, useRef, useState } from "react";
import { StyleSheet, Text, type TextInstance, View } from "react-native";
import {
	failureReport,
	median,
	nextFrame,
	type Phase,
	round3,
	TEXT_COLOR,
	useCommitFrame,
	writeReport,
} from "../arena";
import { Screen } from "../Screen";

const COLUMNS = 15;
const ROWS = 20;
const DURATION_MS = 5000;
const CALIBRATION_FRAMES = 30;

const ROW_INDICES = Array.from({ length: ROWS }, (_, row) => row);
const COLUMN_INDICES = Array.from({ length: COLUMNS }, (_, column) => column);

interface Tick {
	frame: number;
	phase: Phase;
}

/**
 * 300 tiles whose text is `frame + i`, rewritten through `useState` from a
 * `requestAnimationFrame` loop for 5 seconds.
 */
export function Ticker() {
	const [tick, setTick] = useState<Tick>({ frame: 0, phase: "idle" });
	const firstTile = useRef<TextInstance>(null);
	const expectedFinal = useRef<number | null>(null);
	const failure = useRef<string | null>(null);

	// The last update commits with `done`; tile 0 is read back before that frame paints.
	const commit = useCommitFrame(() => {
		const want = expectedFinal.current;
		if (want === null) {
			return;
		}
		const shown = firstTile.current?.textContent;
		if (shown !== String(want)) {
			failure.current = `tile 0 shows ${shown}, expected ${want}`;
			setTick({ frame: want, phase: "failed" });
		}
	});

	useEffect(() => {
		let cancelled = false;

		async function run(): Promise<void> {
			// Measure the refresh period on idle frames, before anything changes.
			const idle: number[] = [];
			let previous = await nextFrame();
			for (let index = 0; index < CALIBRATION_FRAMES; index += 1) {
				const now = await nextFrame();
				idle.push(now - previous);
				previous = now;
			}
			const vsyncMs = median(idle);

			const intervals: number[] = [];
			let frame = 0;
			let start = -1;
			let last = -1;
			await new Promise<void>((resolve) => {
				const onFrame = (timestamp: number): void => {
					if (cancelled) {
						return;
					}
					if (start < 0) {
						start = timestamp;
					} else {
						intervals.push(timestamp - last);
					}
					last = timestamp;
					frame += 1;
					if (timestamp - start >= DURATION_MS) {
						expectedFinal.current = frame;
						void commit(() => setTick({ frame, phase: "done" })).then(() =>
							resolve()
						);
						return;
					}
					setTick({ frame, phase: "running" });
					requestAnimationFrame(onFrame);
				};
				requestAnimationFrame(onFrame);
			});

			if (failure.current !== null) {
				throw new Error(failure.current);
			}
			const skipped = intervals.filter((interval) =>
				interval > vsyncMs * 1.5
			).length;
			writeReport({
				scenario: "ticker",
				lane: "idiomatic",
				ok: true,
				notes: [
					"vsync_ms is the median requestAnimationFrame interval over 30 idle frames before the run",
				],
				metrics: {
					frames_count: frame,
					skipped_count: skipped,
					final_frame: frame,
					vsync_ms: round3(vsyncMs),
					frame_interval_ms: round3(median(intervals)),
				},
			});
		}

		run().catch((error: unknown) => {
			setTick((current) => ({ frame: current.frame, phase: "failed" }));
			writeReport(failureReport("ticker", error));
		});
		return () => {
			cancelled = true;
		};
	}, [commit]);

	return (
		<Screen name="ticker" phase={tick.phase}>
			<View style={styles.grid}>
				{ROW_INDICES.map((row) => (
					<View key={row} style={styles.row}>
						{COLUMN_INDICES.map((column) => {
							const index = row * COLUMNS + column;
							return (
								<Tile
									key={index}
									value={tick.frame + index}
									textRef={index === 0 ? firstTile : undefined}
								/>
							);
						})}
					</View>
				))}
			</View>
		</Screen>
	);
}

const Tile = memo(
	function Tile(
		{ value, textRef }: { value: number; textRef?: Ref<TextInstance> },
	) {
		return (
			<View style={styles.tile}>
				<Text ref={textRef} style={styles.label}>
					{value}
				</Text>
			</View>
		);
	},
);

const styles = StyleSheet.create({
	grid: {
		flex: 1,
	},
	row: {
		flex: 1,
		flexDirection: "row",
	},
	tile: {
		flex: 1,
		alignItems: "center",
		justifyContent: "center",
		borderWidth: StyleSheet.hairlineWidth,
		borderColor: "#DDDDDD",
	},
	label: {
		color: TEXT_COLOR,
		fontSize: 10,
	},
});
