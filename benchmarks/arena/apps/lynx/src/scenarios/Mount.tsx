import {
	runOnBackground,
	runOnMainThread,
	useEffect,
	useMainThreadRef,
	useMemo,
	useRef,
	useState,
} from "@lynx-js/react";
import type { MainThread } from "@lynx-js/types";

import type { ScenarioProps } from "../App.js";
import {
	median,
	nextFrame,
	now,
	PipelineWatcher,
	round3,
	UNIX_CLOCK_SOURCE,
	unixNow,
} from "../arena.js";
import { textOf } from "../elements.js";
import { hasPerformanceObserver } from "./Cold.js";

const CARDS = 1000;
const ROUNDS = 5;
const PIPELINE_TIMEOUT_MS = 30_000;

interface Card {
	id: number;
	title: string;
	subtitle: string;
}

type Step = "create" | "update" | "clear";

interface RenderedCards {
	count: number;
	first: string;
	last: string;
}

function createCards(): Card[] {
	return Array.from({ length: CARDS }, (_, index) => ({
		id: index + 1,
		title: `Card ${index + 1}`,
		subtitle: `Subtitle ${index + 1}`,
	}));
}

/** Every tenth card (10, 20, …, 1000) gets ` !` appended to its title. */
function updateEveryTenth(cards: Card[]): Card[] {
	return cards.map((card) =>
		card.id % 10 === 0 ? { ...card, title: `${card.title} !` } : card
	);
}

function clearCards(): Card[] {
	return [];
}

/** Reads the rendered titles back from the element tree on the main thread. */
function readRendered(container: MainThread.Element | null): RenderedCards {
	"main thread";
	if (container === null) return { count: -1, first: "", last: "" };
	const titles = container.querySelectorAll(".card-title");
	return {
		count: titles.length,
		first: textOf(titles[0]),
		last: textOf(titles[titles.length - 1]),
	};
}

function expected(step: Step): RenderedCards {
	switch (step) {
		case "create":
			return { count: CARDS, first: "Card 1", last: `Card ${CARDS}` };
		case "update":
			return { count: CARDS, first: "Card 1", last: `Card ${CARDS} !` };
		case "clear":
			return { count: 0, first: "", last: "" };
	}
}

/**
 * Big bang: mount 1,000 cards in a plain (non-virtualised) scroll view,
 * retitle every tenth, then clear them, five times. Each step is timed from
 * the state change to the end of the rendering pipeline it started, which
 * Lynx reports through a `PipelineEntry` for the step's `__lynx_timing_flag`
 * once the UI operations were applied and painted.
 */
export function Mount(props: ScenarioProps) {
	const { setPhase, finish } = props;
	const [cards, setCards] = useState<Card[]>([]);
	const [flag, setFlag] = useState("mount-initial");
	const containerRef = useMainThreadRef<MainThread.Element | null>(null);
	const commits = useRef<(() => void)[]>([]);
	const watcher = useMemo(() => {
		"background only";
		return hasPerformanceObserver() ? new PipelineWatcher() : undefined;
	}, []);

	useEffect(() => {
		const wake = commits.current.shift();
		if (wake !== undefined) wake();
	}, [flag]);

	useEffect(() => {
		const rendered = runOnMainThread((): RenderedCards => {
			"main thread";
			return readRendered(containerRef.current);
		});

		// Without pipeline entries (the web), the after-frame hook is the main
		// thread's next animation frame after it applied the commit, reported
		// back to the background thread.
		const mainThreadFrame = () =>
			new Promise<void>((resolve) => {
				const wake = () => resolve();
				void runOnMainThread(() => {
					"main thread";
					requestAnimationFrame(() => {
						runOnBackground(wake)();
					});
				})();
			});

		async function step(
			name: Step,
			round: number,
			next: (cards: Card[]) => Card[],
		): Promise<number> {
			const identifier = `mount-${name}-${round}`;
			const committed = new Promise<void>((resolve) =>
				commits.current.push(resolve)
			);
			const startUnix = unixNow();
			const start = now();
			setCards(next);
			setFlag(identifier);
			setPhase("running");
			let elapsed: number;
			if (watcher !== undefined) {
				const entry = await watcher.waitFor(identifier, PIPELINE_TIMEOUT_MS);
				elapsed = entry.pipelineEnd - startUnix;
			} else {
				await committed;
				await mainThreadFrame();
				elapsed = now() - start;
			}
			const seen = await rendered();
			const want = expected(name);
			if (
				seen.count !== want.count || seen.first !== want.first ||
				seen.last !== want.last
			) {
				throw new Error(
					`${identifier}: rendered ${JSON.stringify(seen)}, expected ${
						JSON.stringify(want)
					}`,
				);
			}
			setPhase("done");
			await nextFrame();
			setPhase("idle");
			await nextFrame();
			return elapsed;
		}

		async function run(): Promise<void> {
			const create: number[] = [];
			const update: number[] = [];
			const clear: number[] = [];
			await nextFrame();
			for (let round = 1; round <= ROUNDS; round += 1) {
				create.push(await step("create", round, createCards));
				update.push(await step("update", round, updateEveryTenth));
				clear.push(await step("clear", round, clearCards));
			}
			const notes = watcher !== undefined
				? [
					`each step is timed from the state change (Unix time from ${UNIX_CLOCK_SOURCE}) to the pipelineEnd of the PipelineEntry for the step's __lynx_timing_flag`,
				]
				: [
					"the web engine raises no pipeline entries: each step is timed from the state change to the main thread's first animation frame after it applied the commit, as seen back on the background thread",
				];
			finish({
				ok: true,
				notes,
				metrics: {
					create_ms: round3(median(create)),
					update_ms: round3(median(update)),
					clear_ms: round3(median(clear)),
				},
				samples: {
					create_ms: create.map(round3),
					update_ms: update.map(round3),
					clear_ms: clear.map(round3),
				},
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

	// Lynx records a flagged pipeline only for an element's first appearance,
	// so each step mounts a fresh, empty marker carrying the step's flag.
	return (
		<view className="fill">
			<view key={flag} className="marker" __lynx_timing_flag={flag} />
			<scroll-view
				className="fill"
				scroll-orientation="vertical"
				main-thread:ref={containerRef}
			>
				{cards.map((card) => (
					<view key={card.id} className="card">
						<text className="card-title">{card.title}</text>
						<text className="card-subtitle">{card.subtitle}</text>
					</view>
				))}
			</scroll-view>
		</view>
	);
}
