import type { ReactNode } from "react";
import { StyleSheet, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { BACKGROUND_COLOR, type Phase, PHASE_COLOR, TEXT_COLOR } from "./arena";

interface ScreenProps {
	/** The status text's first word: the scenario, or `home`. */
	name: string;
	phase: Phase;
	children: ReactNode;
}

/** The contract's root: a safe-area column whose first child is the 48 pt header row. */
export function Screen({ name, phase, children }: ScreenProps) {
	return (
		<SafeAreaView style={styles.root}>
			<View style={styles.header}>
				<View
					style={[styles.beacon, { backgroundColor: PHASE_COLOR[phase] }]}
				/>
				<Text style={styles.status}>
					{name} {phase}
				</Text>
			</View>
			{children}
		</SafeAreaView>
	);
}

const styles = StyleSheet.create({
	root: {
		flex: 1,
		backgroundColor: BACKGROUND_COLOR,
	},
	header: {
		height: 48,
		flexDirection: "row",
		alignItems: "center",
	},
	beacon: {
		width: 32,
		height: 32,
		margin: 8,
	},
	status: {
		color: TEXT_COLOR,
		fontSize: 16,
	},
});
