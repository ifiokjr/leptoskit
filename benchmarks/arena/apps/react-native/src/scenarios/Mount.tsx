import { memo, useEffect, useRef, useState } from "react";
import {
	ScrollView,
	StyleSheet,
	Text,
	View,
	type ViewInstance,
} from "react-native";
import {
	failureReport,
	median,
	type Phase,
	round3,
	TEXT_COLOR,
	useCommitFrame,
	writeReport,
} from "../arena";
import { Screen } from "../Screen";

const ROUNDS = 5;
const CARD_COUNT = 1000;

interface Card {
	id: number;
	title: string;
}

interface Expectation {
	count: number;
	first?: string;
	last?: string;
}

type Step = "create" | "update" | "clear";

function createCards(): Card[] {
	return Array.from(
		{ length: CARD_COUNT },
		(_, index) => ({ id: index + 1, title: `Card ${index + 1}` }),
	);
}

/** Every tenth card (10, 20, …, 1000) gets ` !` appended to its title. */
function updateCards(cards: readonly Card[]): Card[] {
	return cards.map((
		card,
	) => (card.id % 10 === 0 ? { id: card.id, title: `${card.title} !` } : card));
}

function expectation(cards: readonly Card[]): Expectation {
	return {
		count: cards.length,
		first: cards[0]?.title,
		last: cards[cards.length - 1]?.title,
	};
}

/**
 * Five rounds of create, update, and clear over 1,000 cards in a
 * non-virtualised ScrollView. Each step is one frame of `running`, then the
 * state change committed together with `done`, timed from `setState` to the
 * `requestAnimationFrame` scheduled from the commit's layout effect. The
 * rendered rows are checked in that layout effect, before the frame paints,
 * and a failed check turns the beacon to `failed` in the same frame.
 */
export function Mount() {
	const [phase, setPhase] = useState<Phase>("idle");
	const [cards, setCards] = useState<Card[]>([]);
	const list = useRef<ViewInstance>(null);
	const expected = useRef<Expectation | null>(null);
	const failure = useRef<string | null>(null);

	const commit = useCommitFrame(() => {
		const want = expected.current;
		if (want === null) {
			return;
		}
		expected.current = null;
		const rows = list.current?.children;
		const count = rows?.length ?? 0;
		const first = rows?.item(0)?.children.item(0)?.textContent;
		const last = rows?.item(count - 1)?.children.item(0)?.textContent;
		if (count !== want.count || first !== want.first || last !== want.last) {
			failure.current =
				`expected ${want.count} rows (${want.first} … ${want.last}), rendered ${count} (${first} … ${last})`;
			setPhase("failed");
		}
	});

	useEffect(() => {
		const durations: Record<Step, number[]> = {
			create: [],
			update: [],
			clear: [],
		};

		let stepsRun = 0;

		async function step(name: Step, next: Card[]): Promise<void> {
			// One idle frame between steps; the last step ends on done.
			if (stepsRun > 0) {
				await commit(() => setPhase("idle"));
			}
			stepsRun += 1;
			await commit(() => setPhase("running"));
			expected.current = expectation(next);
			const start = performance.now();
			const end = await commit(() => {
				setCards(next);
				setPhase("done");
			});
			durations[name].push(round3(end - start));
			if (failure.current !== null) {
				throw new Error(`${name}: ${failure.current}`);
			}
		}

		async function run(): Promise<void> {
			for (let round = 0; round < ROUNDS; round += 1) {
				const created = createCards();
				await step("create", created);
				await step("update", updateCards(created));
				await step("clear", []);
			}
			writeReport({
				scenario: "mount",
				lane: "idiomatic",
				ok: true,
				notes: [
					"each step is timed from setState to the requestAnimationFrame scheduled from the commit's useLayoutEffect",
				],
				metrics: {
					create_ms: median(durations.create),
					update_ms: median(durations.update),
					clear_ms: median(durations.clear),
				},
				samples: {
					create_ms: durations.create,
					update_ms: durations.update,
					clear_ms: durations.clear,
				},
			});
		}

		run().catch((error: unknown) => {
			setPhase("failed");
			writeReport(failureReport("mount", error));
		});
	}, [commit]);

	return (
		<Screen name="mount" phase={phase}>
			<ScrollView style={styles.scroll}>
				<View ref={list}>
					{cards.map((card) => <CardRow key={card.id} card={card} />)}
				</View>
			</ScrollView>
		</Screen>
	);
}

const CardRow = memo(function CardRow({ card }: { card: Card }) {
	return (
		<View style={styles.card}>
			<Text style={styles.title}>{card.title}</Text>
			<Text style={styles.subtitle}>Subtitle {card.id}</Text>
		</View>
	);
});

const styles = StyleSheet.create({
	scroll: {
		flex: 1,
	},
	card: {
		paddingHorizontal: 16,
		paddingVertical: 6,
		borderBottomWidth: StyleSheet.hairlineWidth,
		borderBottomColor: "#DDDDDD",
	},
	title: {
		color: TEXT_COLOR,
		fontSize: 15,
	},
	subtitle: {
		color: TEXT_COLOR,
		fontSize: 12,
	},
});
