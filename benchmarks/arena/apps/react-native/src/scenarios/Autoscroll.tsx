import { useEffect, useRef, useState } from "react";
import {
	FlatList,
	type NativeScrollEvent,
	type NativeSyntheticEvent,
	StyleSheet,
} from "react-native";
import {
	failureReport,
	nextFrame,
	type Phase,
	round3,
	writeReport,
} from "../arena";
import { FEED, feedItemLayout, feedKey, renderFeedItem } from "../Feed";
import { Screen } from "../Screen";

const DURATION_MS = 5000;
const SPEED_PT_PER_MS = 1.5;
/** How long the final offset may take to come back through `onScroll`. */
const SETTLE_FRAMES = 60;

/**
 * Scrolls the feed by app logic: one `scrollToOffset({animated: false})` per
 * `requestAnimationFrame`, advancing 1.5 pt per elapsed millisecond.
 */
export function Autoscroll() {
	const [phase, setPhase] = useState<Phase>("idle");
	const list = useRef<FlatList<number>>(null);
	const reportedOffset = useRef(0);

	useEffect(() => {
		let cancelled = false;

		async function run(): Promise<void> {
			// Let the first screenful of items mount before the clock starts.
			await nextFrame();
			await nextFrame();

			let frames = 0;
			let offset = 0;
			await new Promise<void>((resolve) => {
				let start = -1;
				let last = -1;
				const onFrame = (timestamp: number): void => {
					if (cancelled) {
						return;
					}
					if (start < 0) {
						start = timestamp;
						last = timestamp;
					}
					offset += SPEED_PT_PER_MS * (timestamp - last);
					last = timestamp;
					frames += 1;
					list.current?.scrollToOffset({ offset, animated: false });
					if (timestamp - start >= DURATION_MS) {
						setPhase("done");
						resolve();
						return;
					}
					if (frames === 1) {
						setPhase("running");
					}
					requestAnimationFrame(onFrame);
				};
				requestAnimationFrame(onFrame);
			});

			for (
				let index = 0;
				index < SETTLE_FRAMES && Math.abs(reportedOffset.current - offset) > 1;
				index += 1
			) {
				await nextFrame();
			}
			if (Math.abs(reportedOffset.current - offset) > 1) {
				throw new Error(
					`scrolled to ${reportedOffset.current}, expected ${offset}`,
				);
			}
			writeReport({
				scenario: "autoscroll",
				lane: "idiomatic",
				ok: true,
				metrics: { frames_count: frames, final_offset: round3(offset) },
			});
		}

		run().catch((error: unknown) => {
			setPhase("failed");
			writeReport(failureReport("autoscroll", error));
		});
		return () => {
			cancelled = true;
		};
	}, []);

	const onScroll = (event: NativeSyntheticEvent<NativeScrollEvent>): void => {
		reportedOffset.current = event.nativeEvent.contentOffset.y;
	};

	return (
		<Screen name="autoscroll" phase={phase}>
			<FlatList
				ref={list}
				style={styles.list}
				data={FEED}
				keyExtractor={feedKey}
				renderItem={renderFeedItem}
				getItemLayout={feedItemLayout}
				onScroll={onScroll}
				scrollEventThrottle={16}
			/>
		</Screen>
	);
}

const styles = StyleSheet.create({
	list: {
		flex: 1,
	},
});
