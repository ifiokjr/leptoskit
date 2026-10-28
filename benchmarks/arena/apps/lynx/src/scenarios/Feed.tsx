import { useEffect, useRef } from "@lynx-js/react";
import type { NodesRef } from "@lynx-js/types";

import type { ScenarioProps } from "../App.js";
import { nextFrame, now, round3, sleep } from "../arena.js";

const ITEMS = 2000;
const ITEM_HEIGHT = 72;
const SCROLL_DURATION_MS = 5000;
/** 1.5 pt per millisecond. */
const SCROLL_SPEED = 1.5;
const BUSY_TARGET_INDEX = 1500;
const BUSY_PERIOD_MS = 100;
const BUSY_BLOCK_MS = 40;
const BUSY_TIMEOUT_MS = 30_000;

interface FeedItem {
	key: string;
	title: string;
	body: string;
	colour: string;
}

const FEED: FeedItem[] = Array.from({ length: ITEMS }, (_, index) => {
	const sentence = `This is the body of item ${index}.`;
	return {
		key: `item-${index}`,
		title: `Item ${index}`,
		body: `${sentence} ${sentence}`,
		colour: `hsl(${(index * 37) % 360}, 60%, 50%)`,
	};
});

/** The 2,000-item virtualised feed: Lynx's `<list>` with fixed 72 pt items. */
function Feed(props: { listRef: { current: NodesRef | null } }) {
	return (
		<list
			id="feed"
			ref={props.listRef}
			className="fill"
			list-type="single"
			span-count={1}
			scroll-orientation="vertical"
		>
			{FEED.map((item) => (
				<list-item
					key={item.key}
					item-key={item.key}
					estimated-main-axis-size-px={ITEM_HEIGHT}
				>
					<view className="feed-item">
						<view className="feed-text">
							<text className="feed-title">{item.title}</text>
							<text className="feed-body" text-maxline="2">{item.body}</text>
						</view>
						<view
							className="feed-square"
							style={{ backgroundColor: item.colour }}
						/>
					</view>
				</list-item>
			))}
		</list>
	);
}

interface ScrollParams {
	position: number;
	offset: number;
	smooth: boolean;
}

/** A UI method call the platform answered with a failure code. */
class UIMethodError extends Error {
	constructor(method: string, readonly result: unknown) {
		super(`${method} failed: ${JSON.stringify(result)}`);
	}

	/**
	 * The element does not implement the method: METHOD_NOT_FOUND (3) on the
	 * native engines; web-core answers UNKNOWN (1) with no data instead.
	 */
	get unsupported(): boolean {
		if (typeof this.result !== "object" || this.result === null) return false;
		const code: unknown = Reflect.get(this.result, "code");
		const data: unknown = Reflect.get(this.result, "data");
		return code === 3 || (code === 1 && (data === "" || data === undefined));
	}
}

/** Calls one of the list's UI methods from the background thread. */
function invoke(
	list: NodesRef,
	method: string,
	params: Record<string, unknown> = {},
): Promise<unknown> {
	return new Promise((resolve, reject) => {
		list.invoke({
			method,
			params,
			success: (result: unknown) => resolve(result),
			fail: (result: unknown) => reject(new UIMethodError(method, result)),
		}).exec();
	});
}

function scrollTo(list: NodesRef, params: ScrollParams): Promise<unknown> {
	return invoke(list, "scrollToPosition", { ...params, alignTo: "top" });
}

function numberField(value: unknown, field: string): number | undefined {
	const found: unknown = typeof value === "object" && value !== null
		? Reflect.get(value, field)
		: undefined;
	return typeof found === "number" ? found : undefined;
}

/**
 * The list's vertical content offset: `getScrollInfo` on iOS and Android;
 * the web list implements `getScrollContainerInfo` instead.
 */
async function scrollOffset(list: NodesRef): Promise<number> {
	try {
		const info = await invoke(list, "getScrollInfo");
		const offset = numberField(info, "scrollY");
		if (offset === undefined) {
			throw new Error(`getScrollInfo returned ${JSON.stringify(info)}`);
		}
		return offset;
	} catch (error: unknown) {
		if (!(error instanceof UIMethodError && error.unsupported)) throw error;
		const info = await invoke(list, "getScrollContainerInfo");
		const offset = numberField(info, "scrollTop");
		if (offset === undefined) {
			throw new Error(
				`getScrollContainerInfo returned ${JSON.stringify(info)}`,
			);
		}
		return offset;
	}
}

/**
 * How autoscroll moves the list each frame: by the frame's step with
 * `scrollBy` (iOS, Android), or to the absolute offset with
 * `scrollToPosition` where the list has no `scrollBy` (the web). The iOS
 * list ignores `scrollToPosition`'s offset for position 0, so no single
 * method covers every platform.
 */
type ScrollMode = "scrollBy" | "scrollToPosition";

async function chooseScrollMode(list: NodesRef): Promise<ScrollMode> {
	try {
		await invoke(list, "scrollBy", { offset: 0 });
		return "scrollBy";
	} catch (error: unknown) {
		if (error instanceof UIMethodError && error.unsupported) {
			return "scrollToPosition";
		}
		throw error;
	}
}

/** Waits until the list has laid out its first cells, so its UI methods act on content. */
async function waitForCells(list: NodesRef): Promise<void> {
	const deadline = now() + 30_000;
	let cells: unknown;
	while (now() < deadline) {
		cells = await invoke(list, "getVisibleCells");
		// iOS and the web answer `{ attachedCells: [...] }`; Android answers the array.
		const attached: unknown =
			Array.isArray(cells) || typeof cells !== "object" || cells === null
				? cells
				: Reflect.get(cells, "attachedCells");
		if (Array.isArray(attached) && attached.length > 0) return;
		await nextFrame();
	}
	throw new Error(
		`the feed list laid out no cells within 30 s (getVisibleCells: ${
			JSON.stringify(cells)
		})`,
	);
}

function errorResult(error: unknown) {
	return {
		ok: false,
		error: error instanceof Error ? error.message : String(error),
		metrics: {},
	};
}

/**
 * Autoscroll: on every background-thread frame callback for 5 s, scroll the
 * list without animation to `offset += 1.5 × dt` through one of its UI
 * methods (see `ScrollMode`).
 */
export function Autoscroll(props: ScenarioProps) {
	const { setPhase, finish } = props;
	const listRef = useRef<NodesRef | null>(null);

	useEffect(() => {
		async function run(): Promise<void> {
			await nextFrame();
			const list = listRef.current;
			if (list === null) throw new Error("the feed list has no ref");
			await waitForCells(list);
			const mode = await chooseScrollMode(list);
			let offset = 0;
			let sent = 0;
			let frames = 0;
			let previous = now();
			const start = previous;
			let finished = false;
			let scrollError: unknown;
			const keepError = (error: unknown) => {
				scrollError ??= error;
			};
			while (!finished) {
				await nextFrame();
				if (scrollError !== undefined) throw scrollError;
				const at = now();
				offset += SCROLL_SPEED * (at - previous);
				previous = at;
				frames += 1;
				finished = at - start >= SCROLL_DURATION_MS;
				if (mode === "scrollToPosition") {
					scrollTo(list, { position: 0, offset, smooth: false }).catch(
						keepError,
					);
				} else {
					invoke(list, "scrollBy", { offset: offset - sent }).catch(keepError);
					sent = offset;
				}
				if (frames === 1) setPhase("running");
			}
			// Let the last scroll land before checking.
			await sleep(300);
			if (scrollError !== undefined) throw scrollError;
			const actual = await scrollOffset(list);
			// Each scroll lands on a whole device pixel, so with scrollBy the list
			// drifts from the requested offset by under a point per frame.
			if (Math.abs(actual - offset) > frames + 2) {
				throw new Error(
					`the list is at ${round3(actual)}, expected ${round3(offset)}`,
				);
			}
			finish({
				ok: true,
				metrics: {
					frames_count: frames,
					final_offset: round3(actual),
					requested_offset: round3(offset),
				},
				notes: [
					`each frame invokes the list's ${mode} UI method from a background-thread requestAnimationFrame callback; final_offset is the list's own scroll offset`,
				],
			});
		}

		run().catch((error: unknown) => finish(errorResult(error)));
	}, []);

	return <Feed listRef={listRef} />;
}

function busyWait(ms: number): void {
	const until = now() + ms;
	while (now() < until) {
		// Hold the background thread, where ReactLynx state and logic run.
	}
}

/**
 * Busy hands: start the list's native animated scroll to item 1,500, then
 * block the background (logic) thread for 40 ms of every 100 ms until the
 * list reports it arrived. Scrolling runs on the platform UI thread.
 */
export function Busy(props: ScenarioProps) {
	const { setPhase, finish } = props;
	const listRef = useRef<NodesRef | null>(null);

	useEffect(() => {
		async function run(): Promise<void> {
			await nextFrame();
			const list = listRef.current;
			if (list === null) throw new Error("the feed list has no ref");
			await waitForCells(list);
			const target = BUSY_TARGET_INDEX * ITEM_HEIGHT;
			setPhase("running");
			await scrollTo(list, {
				position: BUSY_TARGET_INDEX,
				offset: 0,
				smooth: true,
			});
			const start = now();
			let blocks = 0;
			let offset = 0;
			while (offset < target - 1) {
				if (now() - start >= BUSY_TIMEOUT_MS) {
					throw new Error(
						`the list stopped at ${round3(offset)}, expected ${target}`,
					);
				}
				busyWait(BUSY_BLOCK_MS);
				blocks += 1;
				await sleep(BUSY_PERIOD_MS - BUSY_BLOCK_MS);
				offset = await scrollOffset(list);
			}
			finish({
				ok: true,
				metrics: {
					blocks_count: blocks,
					duration_ms: round3(now() - start),
					final_offset: round3(offset),
				},
				notes: [
					"the animated scroll is the list's scrollToPosition with smooth: true; arrival is read from the list's scroll offset between blocks",
				],
			});
		}

		run().catch((error: unknown) => finish(errorResult(error)));
	}, []);

	return <Feed listRef={listRef} />;
}
