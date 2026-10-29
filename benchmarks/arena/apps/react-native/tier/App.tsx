// Placeholder: scripts/arena/emit/react-native.ts overwrites this file and
// writes src/screens/ when it generates a tier.
import { Text } from "react-native";
import { SafeAreaProvider, SafeAreaView } from "react-native-safe-area-context";

export function App() {
	return (
		<SafeAreaProvider>
			<SafeAreaView>
				<Text>Arena tier shell</Text>
			</SafeAreaView>
		</SafeAreaProvider>
	);
}
