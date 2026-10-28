// The feed shared by `autoscroll` and `busy`: 2,000 cards, 72 pt each, in a FlatList.

import { memo } from "react";
import { StyleSheet, Text, View } from "react-native";
import { TEXT_COLOR } from "./arena";

export const FEED_LENGTH = 2000;
export const ITEM_HEIGHT = 72;

export const FEED: readonly number[] = Array.from(
	{ length: FEED_LENGTH },
	(_, index) => index,
);

export function feedKey(item: number): string {
	return String(item);
}

export function feedItemLayout(
	_data: ArrayLike<number> | null | undefined,
	index: number,
) {
	return { length: ITEM_HEIGHT, offset: ITEM_HEIGHT * index, index };
}

export function renderFeedItem({ item }: { item: number }) {
	return <FeedItem index={item} />;
}

const FeedItem = memo(function FeedItem({ index }: { index: number }) {
	const body = `This is the body of item ${index}.`;
	return (
		<View style={styles.item}>
			<View
				style={[styles.square, {
					backgroundColor: `hsl(${(index * 37) % 360}, 60%, 50%)`,
				}]}
			/>
			<View style={styles.text}>
				<Text style={styles.title}>Item {index}</Text>
				<Text style={styles.body} numberOfLines={2}>
					{body} {body}
				</Text>
			</View>
		</View>
	);
});

const styles = StyleSheet.create({
	item: {
		height: ITEM_HEIGHT,
		flexDirection: "row",
		alignItems: "center",
		paddingHorizontal: 16,
		overflow: "hidden",
	},
	square: {
		width: 40,
		height: 40,
		marginRight: 12,
	},
	text: {
		flex: 1,
	},
	title: {
		color: TEXT_COLOR,
		fontSize: 15,
		fontWeight: "600",
	},
	body: {
		color: TEXT_COLOR,
		fontSize: 12,
	},
});
