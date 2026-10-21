//! The tablebench client: the leptoskit runtime benchmark app.
//!
//! A keyed table with the js-framework-benchmark scenario set — create rows,
//! partial update, swap, remove, select, append, clear — plus an increment
//! button for click throughput. Every scenario is a real DOM button whose
//! handler mutates signals, so the measured path is the one a user hits.
//!
//! Each row's label is its own signal: the keyed list reuses nodes by key
//! and does not re-run children for unchanged keys, so fine-grained updates
//! ride the row's own reactivity instead of rebuilding the list. Labels come
//! from the shared `bench.js` glue so every framework renders byte-identical
//! text for the same row id.

use leptos::mount::mount_to_body;
use leptos::prelude::*;
use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen]
extern "C" {
	#[wasm_bindgen(js_namespace = ["window", "__bench"], js_name = labelFor)]
	fn label_for(id: u32) -> String;

	#[wasm_bindgen(js_namespace = ["window", "__bench"])]
	fn mounted();
}

/// One keyed row: a stable id and a fine-grained label signal.
#[derive(Clone, Copy)]
struct Row {
	id: u32,
	label: RwSignal<String>,
}

/// Rendered into the DOM so the build-speed benchmark's incremental scenario
/// can rewrite it and produce a genuinely different wasm module: a comment
/// edit would compile to identical bytes and the pipeline would, correctly,
/// skip wasm-bindgen.
const BENCH_STAMP: &str = "stamp-0";

/// Builds `count` rows starting at `start`, with labels from the shared glue.
fn build_rows(start: u32, count: u32) -> Vec<Row> {
	(start..start + count)
		.map(|id| {
			Row {
				id,
				label: RwSignal::new(label_for(id)),
			}
		})
		.collect()
}

/// The benchmark interface.
#[component]
fn TableBench() -> impl IntoView {
	let rows = RwSignal::new(Vec::<Row>::new());
	let next_id = RwSignal::new(1_u32);
	let selected = RwSignal::new(None::<u32>);
	let count = RwSignal::new(0_u32);

	let create = move |total: u32| {
		rows.set(build_rows(next_id.get(), total));
		next_id.update(|id| *id += total);
	};
	let append = move |total: u32| {
		rows.update(|current| current.extend(build_rows(next_id.get(), total)));
		next_id.update(|id| *id += total);
	};
	let update_every_10th = move |_| {
		rows.update(|current| {
			for row in current.iter().step_by(10) {
				row.label.update(|label| {
					*label = format!("{label} !!!");
				});
			}
		});
	};
	let swap_rows = move |_| {
		rows.update(|current| {
			if current.len() > 998 {
				current.swap(1, 998);
			}
		});
	};
	let remove_row = move |_| {
		rows.update(|current| {
			if current.len() > 500 {
				current.remove(500);
			}
		});
	};
	let select_row = move |_| {
		selected.set(rows.with(|current| current.get(42).map(|row| row.id)));
	};
	let selected_label = move || {
		selected
			.get()
			.and_then(|id| {
				rows.get()
					.iter()
					.find(|row| row.id == id)
					.map(|row| row.label.get())
			})
			.unwrap_or_else(|| "none".to_owned())
	};

	view! {
		<div id="controls" data-stamp=BENCH_STAMP>
			<button id="create-1000" on:click=move |_| create(1000)>
				"create 1000"
			</button>
			<button id="create-10000" on:click=move |_| create(10000)>
				"create 10000"
			</button>
			<button id="append-1000" on:click=move |_| append(1000)>
				"append 1000"
			</button>
			<button id="update-every-10th" on:click=update_every_10th>
				"update every 10th"
			</button>
			<button id="swap-rows" on:click=swap_rows>
				"swap rows"
			</button>
			<button id="remove-row" on:click=remove_row>
				"remove row"
			</button>
			<button id="select-row" on:click=select_row>
				"select row"
			</button>
			<button id="clear" on:click=move |_| rows.set(Vec::new())>
				"clear"
			</button>
			<button id="increment" on:click=move |_| count.update(|value| *value += 1)>
				"increment"
			</button>
			<span id="count-label">"count: " {move || count.get()}</span>
			<span id="selected-label">"selected: " {selected_label}</span>
		</div>
		<table>
			<tbody id="tbody">
				<For each=move || rows.get() key=|row| row.id let(row)>
					<tr
						data-id=row.id
						class="row"
						class:selected=move || selected.get() == Some(row.id)
					>
						<td class="col-id">{row.id}</td>
						<td class="col-label" on:click=move |_| selected.set(Some(row.id))>
							{move || row.label.get()}
						</td>
					</tr>
				</For>
			</tbody>
		</table>
	}
}

/// Boots the app and marks the interface mounted for the startup metric.
#[wasm_bindgen(start)]
fn start() {
	mount_to_body(|| view! { <TableBench /> });
	mounted();
}

// bench bump

// bench bump

// bench bump

// bench bump

// bench bump

// bench bump

// bench bump

// bench bump
