// The tablebench vanilla-js app: hand-rolled DOM manipulation, the floor
// every framework is measured against. Same scenario buttons, same labels,
// same DOM shape as the leptoskit and React fixtures.
const state = {
	rows: [],
	nextId: 1,
	selected: null,
	count: 0,
};

const tbody = document.createElement("tbody");
tbody.id = "tbody";

function renderRow(row) {
	const tr = document.createElement("tr");
	tr.className = "row";
	tr.dataset.id = row.id;

	const id = document.createElement("td");
	id.className = "col-id";
	id.textContent = row.id;

	const label = document.createElement("td");
	label.className = "col-label";
	label.textContent = row.label;
	label.addEventListener("click", () => select(row.id));

	tr.append(id, label);
	return tr;
}

function syncSelected() {
	for (const tr of tbody.children) {
		tr.classList.toggle("selected", Number(tr.dataset.id) === state.selected);
	}
	document.getElementById("selected-label").textContent = `selected: ${
		state.rows.find((row) => row.id === state.selected)?.label ?? "none"
	}`;
}

function buildRows(start, total) {
	const rows = [];
	for (let id = start; id < start + total; id += 1) {
		rows.push({ id, label: window.__bench.labelFor(id) });
	}
	return rows;
}

function setRows(rows) {
	tbody.replaceChildren(...rows.map(renderRow));
	state.rows = rows;
}

function select(id) {
	state.selected = id;
	syncSelected();
}

const controls = document.createElement("div");
controls.id = "controls";
const button = (id, text, onClick) => {
	const element = document.createElement("button");
	element.id = id;
	element.textContent = text;
	element.addEventListener("click", onClick);
	return element;
};
const span = (id) => {
	const element = document.createElement("span");
	element.id = id;
	return element;
};
const countLabel = span("count-label");
const selectedLabel = span("selected-label");

controls.append(
	button("create-1000", "create 1000", () => {
		setRows(buildRows(state.nextId, 1000));
		state.nextId += 1000;
	}),
	button("create-10000", "create 10000", () => {
		setRows(buildRows(state.nextId, 10000));
		state.nextId += 10000;
	}),
	button("append-1000", "append 1000", () => {
		for (const row of buildRows(state.nextId, 1000)) {
			tbody.append(renderRow(row));
			state.rows.push(row);
		}
		state.nextId += 1000;
	}),
	button("update-every-10th", "update every 10th", () => {
		// Collect the cells before mutating: indexing a live collection while
		// writing to it invalidates its cache on every write.
		const cells = tbody.querySelectorAll(".col-label");
		for (let index = 0; index < state.rows.length; index += 10) {
			const row = state.rows[index];
			row.label = `${row.label} !!!`;
			cells[index].textContent = row.label;
		}
	}),
	button("swap-rows", "swap rows", () => {
		if (state.rows.length <= 998) return;
		const [first, second] = [state.rows[1], state.rows[998]];
		state.rows[1] = second;
		state.rows[998] = first;
		const [firstNode, secondNode] = [tbody.children[1], tbody.children[998]];
		firstNode.querySelector(".col-label").textContent = state.rows[1].label;
		firstNode.dataset.id = state.rows[1].id;
		secondNode.querySelector(".col-label").textContent = state.rows[998].label;
		secondNode.dataset.id = state.rows[998].id;
	}),
	button("remove-row", "remove row", () => {
		if (state.rows.length <= 500) return;
		state.rows.splice(500, 1);
		tbody.children[500].remove();
	}),
	button("select-row", "select row", () => {
		select(state.rows[42]?.id ?? null);
	}),
	button("clear", "clear", () => setRows([])),
	button("increment", "increment", () => {
		state.count += 1;
		countLabel.textContent = `count: ${state.count}`;
	}),
	countLabel,
	selectedLabel,
);
countLabel.textContent = "count: 0";
selectedLabel.textContent = "selected: none";

const table = document.createElement("table");
table.append(tbody);
document.getElementById("app").replaceChildren(controls, table);

window.__bench.mounted();
