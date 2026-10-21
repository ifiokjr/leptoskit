// The shared benchmark contract every runtime fixture implements.
//
// Load this script before the app. It provides:
//
// - `__bench.mounted()` — the app calls it once its interface is in the DOM;
// - `__bench.labelFor(id)` — the one source of truth for row labels, so every
//   framework renders byte-identical text for the same row id;
// - `__bench.run(buttonId, isDone)` — on an animation frame boundary, click a
//   scenario button and resolve with the duration once the caller's
//   predicate holds and the page is laid out, so a broken app can never
//   record a fast number: it times out instead;
// - `__bench.clicks(n)` — on an animation frame boundary, dispatch n real
//   clicks on the increment button and resolve once the rendered count
//   reaches n and the page is laid out;
// - DOM readers (`rows`, `label`, `rowClass`, `count`, ...) the harness uses
//   to verify state before and after every scenario. Apps whose renderer
//   emits something other than `tr`/`td` (canvas or block elements) override
//   `__bench.read` at boot.
(() => {
	"use strict";

	const ADJECTIVES = [
		"stormy",
		"quiet",
		"rapid",
		"sturdy",
		"clever",
		"eager",
		"fierce",
		"gentle",
		"jolly",
		"lively",
		"novel",
		"polite",
		"proud",
		"brave",
		"calm",
		"eager",
		"swift",
		"tender",
		"vivid",
		"warm",
		"wise",
		"zesty",
		"bright",
		"crisp",
		"deep",
		"fair",
		"grand",
		"humble",
		"keen",
		"lucid",
		"merry",
		"noble",
	];
	const NOUNS = [
		"otter",
		"forest",
		"harbor",
		"meadow",
		"summit",
		"river",
		"canyon",
		"glacier",
		"lantern",
		"compass",
		"anchor",
		"beacon",
		"cascade",
		"dawn",
		"ember",
		"fjord",
		"grove",
		"horizon",
		"island",
		"juniper",
		"kettle",
		"lagoon",
		"marsh",
		"nimbus",
		"orchard",
		"prairie",
		"quarry",
		"ridge",
		"sparrow",
		"timber",
		"umbra",
		"valley",
	];

	// Resolves inside an animation frame callback, so every measurement starts
	// at the same point of a frame.
	const frameStart = () => new Promise((resolve) => requestAnimationFrame(() => resolve()));

	// Forces style and layout, so a measurement includes the browser's work for
	// the DOM the app just produced.
	const layout = () => document.documentElement.getBoundingClientRect();

	// Yields one task without waiting for a frame: pending microtasks and any
	// task the app scheduled run first, and the browser still renders between
	// tasks when a frame is due, so apps that render inside a frame callback
	// finish too. A channel message is not clamped like a nested timer.
	const channel = new MessageChannel();
	const nextTask = () =>
		new Promise((resolve) => {
			channel.port1.onmessage = () => resolve();
			channel.port2.postMessage(null);
		});

	// Resolves with the milliseconds since `start` once `isDone()` holds on a
	// laid-out page. The end is not a frame boundary: ending on frames measured
	// whole frames, and work of a millisecond or two read as one frame or two
	// depending on where Chromium's begin-frame fell, so identical builds split
	// their samples between ~17 ms and ~33 ms and the median flipped between
	// them (`table.clear` and `table.update-every-10th` on #97 and #99).
	const measure = async (start, isDone, what) => {
		const deadline = start + 30_000;
		while (performance.now() < deadline) {
			layout();
			if (await isDone()) {
				return performance.now() - start;
			}
			await nextTask();
		}
		throw new Error(`${what} did not finish within 30s`);
	};

	const tbody = () => document.getElementById("tbody");

	const defaultRead = {
		rows: () => tbody().querySelectorAll("tr").length,
		row: (position) => tbody().children[position] ?? null,
		label: (row) => {
			const cell = row?.querySelector(".col-label");
			return cell ? cell.textContent : null;
		},
		rowClass: (row) => (row ? row.className : null),
		rowId: (row) => (row ? Number(row.dataset.id) : null),
	};

	const api = {
		ready: false,
		mountedAt: null,
		read: defaultRead,

		mounted() {
			if (api.ready) return;
			api.ready = true;
			api.mountedAt = performance.now();
		},

		labelFor(id) {
			const adjective = ADJECTIVES[(id * 7) % ADJECTIVES.length];
			const noun = NOUNS[(id * 13) % NOUNS.length];
			return `${adjective} ${noun} ${id}`;
		},

		rows() {
			return api.read.rows();
		},
		label(position) {
			return api.read.label(api.read.row(position));
		},
		rowClass(position) {
			return api.read.rowClass(api.read.row(position));
		},
		rowId(position) {
			return api.read.rowId(api.read.row(position));
		},
		count() {
			const text = document.getElementById("count-label")?.textContent ?? "";
			const match = text.match(/\d+/);
			return match ? Number(match[0]) : null;
		},
		selectedText() {
			return document.getElementById("selected-label")?.textContent ?? null;
		},
		heapBytes() {
			if (!performance.memory) return null;
			// Collect first so the reading is live memory, not whatever garbage
			// the last scenario (or the previous page in this renderer) left
			// behind. The harness launches Chromium with `--expose-gc`.
			if (typeof globalThis.gc === "function") globalThis.gc();
			return performance.memory.usedJSHeapSize;
		},
		transferBytes() {
			let total = 0;
			for (const entry of performance.getEntriesByType("resource")) {
				total += entry.encodedBodySize || 0;
			}
			const navigation = performance.getEntriesByType("navigation")[0];
			if (navigation) {
				total += navigation.encodedBodySize || 0;
			}
			return total;
		},

		async run(buttonId, isDone) {
			const button = document.getElementById(buttonId);
			if (!button) {
				throw new Error(`missing scenario button #${buttonId}`);
			}
			await frameStart();
			const start = performance.now();
			button.click();
			return measure(start, isDone, `scenario ${buttonId}`);
		},

		async clicks(total) {
			const button = document.getElementById("increment");
			if (!button) {
				throw new Error("missing #increment button");
			}
			await frameStart();
			const start = performance.now();
			const click = new MouseEvent("click", { bubbles: true, cancelable: true });
			for (let index = 0; index < total; index += 1) {
				button.dispatchEvent(click);
			}
			return measure(start, () => api.count() === total, `increment to ${total}`);
		},
	};

	window.__bench = api;
})();
