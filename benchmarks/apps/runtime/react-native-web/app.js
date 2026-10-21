// The tablebench react-native-web app: real React Native source — the same
// imports an RN app uses — rendered to the DOM by react-native-web through a
// vendored bundle. It exercises React Native's view model (View/Text/StyleSheet)
// rather than the table markup of the other fixtures, so it overrides the
// DOM readers at boot.
import { createElement as h, useState } from "react";
import { AppRegistry, StyleSheet, Text, View } from "react-native";

let nextId = 1;

function buildRows(start, total) {
	const rows = [];
	for (let id = start; id < start + total; id += 1) {
		rows.push({ id, label: window.__bench.labelFor(id) });
	}
	return rows;
}

const styles = StyleSheet.create({
	controls: { flexDirection: "row", flexWrap: "wrap", gap: 4 },
	rows: { flexDirection: "column" },
	row: { flexDirection: "row", gap: 8 },
	selected: { backgroundColor: "#ffd" },
	label: { minWidth: 120 },
});

function Row({ row, selected, onSelect }) {
	return h(
		View,
		{
			style: [styles.row, selected && styles.selected],
			dataSet: {
				benchRow: "",
				rowId: String(row.id),
				...(selected ? { selected: "" } : {}),
			},
		},
		h(Text, { dataSet: { benchId: "" } }, row.id),
		h(
			Text,
			{
				style: styles.label,
				dataSet: { benchLabel: "" },
				onClick: () => onSelect(row.id),
			},
			row.label,
		),
	);
}

// Buttons are `View`s with `onClick`: react-native-web's `Pressable`
// drives presses off pointer events and ignores a plain click event, which
// is what the shared harness dispatches to every framework alike.
function button(id, label, onClick) {
	return h(
		View,
		{ key: id, nativeID: id, onClick, role: "button" },
		h(Text, null, label),
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

	const selectedLabel = () =>
		rows.find((row) => row.id === selected)?.label ?? "none";

	return h(
		View,
		null,
		h(
			View,
			{ style: styles.controls },
			button("create-1000", "create 1000", () => create(1000)),
			button("create-10000", "create 10000", () => create(10000)),
			button("append-1000", "append 1000", () => {
				// Built eagerly: React runs state updaters lazily at render
				// time, after `nextId` has already advanced.
				const appended = buildRows(nextId, 1000);
				nextId += 1000;
				setRows((current) => [...current, ...appended]);
			}),
			button(
				"update-every-10th",
				"update every 10th",
				() =>
					setRows((current) =>
						current.map((row, index) =>
							index % 10 === 0 ? { ...row, label: `${row.label} !!!` } : row
						)
					),
			),
			button("swap-rows", "swap rows", () =>
				setRows((current) => {
					if (current.length <= 998) return current;
					const next = [...current];
					[next[1], next[998]] = [next[998], next[1]];
					return next;
				})),
			button(
				"remove-row",
				"remove row",
				() =>
					setRows((current) =>
						current.length <= 500
							? current
							: current.filter((_, index) => index !== 500)
					),
			),
			button(
				"select-row",
				"select row",
				() => setSelected((rows[42] ?? { id: null }).id),
			),
			button("clear", "clear", () => setRows([])),
			button("increment", "increment", () => setCount((value) => value + 1)),
			h(Text, { nativeID: "count-label" }, "count: ", count),
			h(Text, { nativeID: "selected-label" }, "selected: ", selectedLabel()),
		),
		h(
			View,
			{ style: styles.rows, nativeID: "tbody" },
			rows.map((row) =>
				h(Row, {
					key: row.id,
					row,
					selected: selected === row.id,
					onSelect: setSelected,
				})
			),
		),
	);
}

// react-native-web renders View/Text as block elements with data attributes,
// not `tr`/`td`; rebind the shared DOM readers to the attributes this app
// emits so the harness verifies the same state as every other fixture.
window.__bench.read = {
	rows: () => document.querySelectorAll("[data-bench-row]").length,
	row: (position) =>
		document.querySelectorAll("[data-bench-row]")[position] ?? null,
	label: (row) => row?.querySelector("[data-bench-label]")?.textContent ?? null,
	rowClass: (
		row,
	) => (row?.hasAttribute("data-selected") ? "row selected" : "row"),
	rowId: (row) => (row ? Number(row.dataset.rowId) : null),
};

AppRegistry.registerComponent("tablebench", () => App);
AppRegistry.runApplication("tablebench", {
	rootTag: document.getElementById("app"),
});
window.__bench.mounted();
