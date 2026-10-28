import { useEffect, useRef, useState } from "react";
import { StyleSheet, Text, View, type ViewInstance } from "react-native";
import {
	failureReport,
	nextFrame,
	type Phase,
	TEXT_COLOR,
	writeReport,
} from "../arena";
import { Screen } from "../Screen";

const ROWS = Array.from({ length: 20 }, (_, index) => `Row ${index + 1}`);

/**
 * The ready screen, done from its first frame. React Native exposes no
 * first-frame time relative to process start, so the harness's own
 * launch-to-pixel measurement is the only cold number.
 */
export function Cold() {
	const [phase, setPhase] = useState<Phase>("done");
	const rows = useRef<ViewInstance>(null);

	useEffect(() => {
		let cancelled = false;
		void nextFrame().then(() => {
			if (cancelled) {
				return;
			}
			const list = rows.current;
			const count = list?.children.length ?? 0;
			const first = list?.children.item(0)?.textContent;
			const last = list?.children.item(count - 1)?.textContent;
			if (
				count !== ROWS.length || first !== ROWS[0] ||
				last !== ROWS[ROWS.length - 1]
			) {
				setPhase("failed");
				writeReport(
					failureReport(
						"cold",
						`rendered ${count} rows, first ${first}, last ${last}`,
					),
				);
				return;
			}
			writeReport({
				scenario: "cold",
				lane: "idiomatic",
				ok: true,
				notes: [
					"React Native exposes no first-frame time relative to process start",
				],
				metrics: { rows_count: count },
			});
		});
		return () => {
			cancelled = true;
		};
	}, []);

	return (
		<Screen name="cold" phase={phase}>
			<View ref={rows}>
				{ROWS.map((row) => (
					<Text key={row} style={styles.row}>
						{row}
					</Text>
				))}
			</View>
		</Screen>
	);
}

const styles = StyleSheet.create({
	row: {
		color: TEXT_COLOR,
		fontSize: 16,
		paddingHorizontal: 16,
		paddingVertical: 4,
	},
});
