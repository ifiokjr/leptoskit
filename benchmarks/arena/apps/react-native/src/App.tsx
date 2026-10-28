import { useState } from "react";
import { Button, ScrollView, StatusBar, StyleSheet, View } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { parseScenario, type Scenario, SCENARIOS } from "./arena";
import { Autoscroll } from "./scenarios/Autoscroll";
import { Busy } from "./scenarios/Busy";
import { Cold } from "./scenarios/Cold";
import { Echo } from "./scenarios/Echo";
import { Mount } from "./scenarios/Mount";
import { Ticker } from "./scenarios/Ticker";
import { Screen } from "./Screen";

export interface AppProps {
	/** From `--scenario` (iOS), the `scenario` intent extra (Android), or `?scenario=` (web). */
	scenario?: string;
}

export function App({ scenario: initialScenario }: AppProps) {
	const [scenario, setScenario] = useState<Scenario | null>(() =>
		parseScenario(initialScenario)
	);

	return (
		<SafeAreaProvider>
			<StatusBar barStyle="dark-content" />
			{scenario === null
				? <Home onRun={setScenario} />
				: <ScenarioScreen scenario={scenario} />}
		</SafeAreaProvider>
	);
}

function ScenarioScreen({ scenario }: { scenario: Scenario }) {
	switch (scenario) {
		case "cold":
			return <Cold />;
		case "mount":
			return <Mount />;
		case "echo":
			return <Echo />;
		case "ticker":
			return <Ticker />;
		case "autoscroll":
			return <Autoscroll />;
		case "busy":
			return <Busy />;
	}
}

function Home({ onRun }: { onRun: (scenario: Scenario) => void }) {
	return (
		<Screen name="home" phase="idle">
			<ScrollView contentContainerStyle={styles.menu}>
				{SCENARIOS.map((scenario) => (
					<View key={scenario} style={styles.menuItem}>
						<Button
							title={scenario}
							onPress={() =>
								onRun(scenario)}
						/>
					</View>
				))}
			</ScrollView>
		</Screen>
	);
}

const styles = StyleSheet.create({
	menu: {
		padding: 16,
	},
	menuItem: {
		marginBottom: 12,
	},
});
