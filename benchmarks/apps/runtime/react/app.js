// The tablebench React app: the same keyed-table scenario set as the
// leptoskit fixture, written with plain `createElement` so no build step or
// JSX transform sits between the committed source and the browser.
//
// `react-bundle.js` is a vendored, minified ESM bundle exporting both
// `react` and `react-dom/client` (a single React copy, so hooks work).
import { createElement, createRoot, useState } from "./vendor/react-bundle.js";

const h = createElement;

// The next row id, stable across renders like the signals in the other
// fixtures; row ids only ever grow so labels stay deterministic.
let nextId = 1;

function buildRows(start, total) {
	const rows = [];
	for (let id = start; id < start + total; id += 1) {
		rows.push({ id, label: window.__bench.labelFor(id) });
	}
	return rows;
}

function Row({ row, selected, onSelect }) {
	return h(
		"tr",
		{ className: selected ? "row selected" : "row", "data-id": row.id },
		h("td", { className: "col-id" }, row.id),
		h(
			"td",
			{ className: "col-label", onClick: () => onSelect(row.id) },
			row.label,
		),
	);
}

function App() {
	const [rows, setRows] = useState([]);
	const [selected, setSelected] = useState(null);
	const [count, setCount] = useState(0);

	const create = (total) => {
		setRows(buildRows(nextId, total));
		nextId += total;
	};

	const selectedLabel = () => {
		const row = rows.find((candidate) => candidate.id === selected);
		return row ? row.label : "none";
	};

	return h(
		"main",
		null,
		h(
			"div",
			{ id: "controls" },
			h(
				"button",
				{ id: "create-1000", onClick: () => create(1000) },
				"create 1000",
			),
			h(
				"button",
				{ id: "create-10000", onClick: () => create(10000) },
				"create 10000",
			),
			h("button", {
				id: "append-1000",
				onClick: () => {
					// Build eagerly: React runs state updaters lazily at render
					// time, after `nextId` has already advanced.
					const appended = buildRows(nextId, 1000);
					nextId += 1000;
					setRows((current) => [...current, ...appended]);
				},
			}, "append 1000"),
			h("button", {
				id: "update-every-10th",
				onClick: () =>
					setRows((current) =>
						current.map((row, index) =>
							index % 10 === 0 ? { ...row, label: `${row.label} !!!` } : row
						)
					),
			}, "update every 10th"),
			h("button", {
				id: "swap-rows",
				onClick: () =>
					setRows((current) => {
						if (current.length <= 998) return current;
						const next = [...current];
						[next[1], next[998]] = [next[998], next[1]];
						return next;
					}),
			}, "swap rows"),
			h("button", {
				id: "remove-row",
				onClick: () =>
					setRows((current) => {
						if (current.length <= 500) return current;
						return current.filter((_, index) => index !== 500);
					}),
			}, "remove row"),
			h("button", {
				id: "select-row",
				onClick: () => setSelected((rows[42] ?? { id: null }).id),
			}, "select row"),
			h("button", { id: "clear", onClick: () => setRows([]) }, "clear"),
			h("button", {
				id: "increment",
				onClick: () => setCount((value) => value + 1),
			}, "increment"),
			h("span", { id: "count-label" }, "count: ", count),
			h("span", { id: "selected-label" }, "selected: ", selectedLabel()),
		),
		h(
			"table",
			null,
			h(
				"tbody",
				{ id: "tbody" },
				rows.map((row) =>
					h(Row, {
						key: row.id,
						row,
						selected: selected === row.id,
						onSelect: setSelected,
					})
				),
			),
		),
	);
}

createRoot(document.getElementById("app")).render(h(App));
window.__bench.mounted();
