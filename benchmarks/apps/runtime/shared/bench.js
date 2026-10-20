// The shared benchmark contract every runtime fixture implements.
//
// Load this script before the app. It provides:
//
// - `__bench.mounted()` — the app calls it once its interface is in the DOM;
// - `__bench.labelFor(id)` — the one source of truth for row labels, so every
//   framework renders byte-identical text for the same row id;
// - `__bench.run(buttonId, isDone)` — click a scenario button and resolve with
//   the duration once the expected DOM state has settled (two animation
//   frames plus the caller's predicate), so a broken app can never record a
//   fast number: it times out instead;
// - `__bench.clicks(n)` — dispatch n real clicks on the increment button and
//   resolve when the rendered count reaches n;
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

	const settle = () =>
		new Promise((resolve) => {
			requestAnimationFrame(() => {
				requestAnimationFrame(resolve);
			});
		});

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

		settle,

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
			const start = performance.now();
			button.click();
			const deadline = start + 30_000;
			while (performance.now() < deadline) {
				await settle();
				if (await isDone()) {
					return performance.now() - start;
				}
			}
			throw new Error(`scenario ${buttonId} did not settle within 30s`);
		},

		async clicks(total) {
			const button = document.getElementById("increment");
			if (!button) {
				throw new Error("missing #increment button");
			}
			const start = performance.now();
			const click = new MouseEvent("click", { bubbles: true, cancelable: true });
			for (let index = 0; index < total; index += 1) {
				button.dispatchEvent(click);
			}
			const deadline = start + 30_000;
			while (performance.now() < deadline) {
				await settle();
				if (api.count() === total) {
					return performance.now() - start;
				}
			}
			throw new Error(`increment never reached ${total}`);
		},
	};

	window.__bench = api;
})();
