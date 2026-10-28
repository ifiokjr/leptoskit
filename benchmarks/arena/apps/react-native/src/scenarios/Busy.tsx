import { useEffect, useRef, useState } from "react";
import {
	FlatList,
	type NativeScrollEvent,
	type NativeSyntheticEvent,
	StyleSheet,
} from "react-native";
import { failureReport, nextFrame, type Phase, writeReport } from "../arena";
import {
	FEED,
	feedItemLayout,
	feedKey,
	ITEM_HEIGHT,
	renderFeedItem,
} from "../Feed";
import { Screen } from "../Screen";

const TARGET_INDEX = 1500;
const TARGET_OFFSET = TARGET_INDEX * ITEM_HEIGHT;
const BLOCK_MS = 40;
const BLOCK_EVERY_MS = 100;
const TIMEOUT_MS = 15000;

/** Busy-waits on the JS thread, the thread React Native runs app logic on. */
function block(ms: number): void {
	const end = performance.now() + ms;
	while (performance.now() < end) {
		// Spin.
	}
}

/**
 * `scrollToIndex({index: 1500, animated: true})`, which the platform scroll
 * view animates natively, while a `setInterval` blocks the JS thread for
 * 40 ms of every 100 ms. Done once `onScroll` reports the target offset.
 */
export function Busy() {
	const [phase, setPhase] = useState<Phase>("idle");
	const list = useRef<FlatList<number>>(null);
	const reportedOffset = useRef(0);

	useEffect(() => {
		let cancelled = false;
		let interval: number | null = null;

		async function run(): Promise<void> {
			await nextFrame();
			await nextFrame();

			let blocks = 0;
			const start = performance.now();
			setPhase("running");
			list.current?.scrollToIndex({ index: TARGET_INDEX, animated: true });
			interval = setInterval(() => {
				block(BLOCK_MS);
				blocks += 1;
			}, BLOCK_EVERY_MS);

			while (Math.abs(reportedOffset.current - TARGET_OFFSET) > 1) {
				if (cancelled) {
					return;
				}
				if (performance.now() - start > TIMEOUT_MS) {
					throw new Error(
						`the scroll stopped at ${reportedOffset.current}, expected ${TARGET_OFFSET}`,
					);
				}
				await nextFrame();
			}
			clearInterval(interval);
			interval = null;
			setPhase("done");
			writeReport({
				scenario: "busy",
				lane: "idiomatic",
				ok: true,
				metrics: { blocks_count: blocks },
			});
		}

		run().catch((error: unknown) => {
			setPhase("failed");
			writeReport(failureReport("busy", error));
		});
		return () => {
			cancelled = true;
			if (interval !== null) {
				clearInterval(interval);
			}
		};
	}, []);

	const onScroll = (event: NativeSyntheticEvent<NativeScrollEvent>): void => {
		reportedOffset.current = event.nativeEvent.contentOffset.y;
	};

	return (
		<Screen name="busy" phase={phase}>
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
