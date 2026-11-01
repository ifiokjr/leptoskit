//! The channel's task board: three columns, and cards that move.
//!
//! leptoskit has no drag gesture yet, so each card has buttons that move it
//! up, down, or to the next column. Each press runs `move_task`, the same
//! serializable mutation a drop would: two people moving cards at once
//! both land, one on top of the other, and every screen shows the same
//! order.

use huddle_api::COLUMNS;
use huddle_api::TaskCard;
use leptoskit::leptos::prelude::Get;
use leptoskit::leptos::prelude::GetUntracked;
use leptoskit::leptos::prelude::Set;
use leptoskit::prelude::*;
use leptoskit::view;

use crate::huddle::Huddle;

fn title(column: &str) -> &'static str {
	match column {
		"todo" => "To do",
		"doing" => "Doing",
		_ => "Done",
	}
}

#[component]
pub fn Panel(huddle: &Huddle) -> Element {
	let huddle = *huddle;
	let cards = huddle.board.data().unwrap_or_default();
	let columns: Fragment = COLUMNS
		.iter()
		.enumerate()
		.map(|(index, column)| {
			let here: Vec<TaskCard> = cards
				.iter()
				.filter(|card| card.column == *column)
				.cloned()
				.collect();
			view! { <ColumnView huddle=&huddle index=index cards=&here /> }
		})
		.collect();
	let new_task = huddle.new_task.get();

	view! {
		<View class="flex-1 gap-3">
			<View class="flex-row gap-2">
				<TextInput
					class="flex-1 rounded-lg bg-slate-800 px-3 py-2 text-base"
					value=new_task.clone()
					placeholder="A new card"
					accessibility_label="New card title"
					on:change=move |text: &str| huddle.new_task.set(text.to_owned())
				/>
				<Button
					class="rounded-lg bg-indigo-500 px-4 py-2 font-semibold"
					enabled=!new_task.trim().is_empty()
					on:press=move || {
						let title = huddle.new_task.get_untracked();
						huddle.new_task.set(String::new());
						huddle.run(huddle_api::AddTask {
							channel: huddle.selected.get_untracked(),
							title,
						});
					}
				>
					"Add card"
				</Button>
			</View>
			<View class="flex-col gap-3 lg:flex-row">{columns}</View>
		</View>
	}
}

#[component]
fn ColumnView(huddle: &Huddle, index: usize, cards: &[TaskCard]) -> Element {
	let huddle = *huddle;
	let column = COLUMNS[index];
	let count = cards.len();
	let rows: Fragment = cards
		.iter()
		.enumerate()
		.map(|(at, card)| {
			view! { <CardView huddle=&huddle column=index at=at count=count card=card /> }
		})
		.collect();

	view! {
		<View class="flex-1 gap-2 rounded-xl bg-slate-900 p-3" accessibility_label=title(column)>
			<Text class="text-sm font-semibold text-slate-300">
				{format!("{} · {count}", title(column))}
			</Text>
			{rows}
		</View>
	}
}

#[component]
fn CardView(huddle: &Huddle, column: usize, at: usize, count: usize, card: &TaskCard) -> Element {
	let huddle = *huddle;
	let task = card.id;
	let to = move |column: usize, index: usize| {
		move || {
			huddle.run(huddle_api::MoveTask {
				task,
				column: COLUMNS[column].to_owned(),
				index: u32::try_from(index).unwrap_or(u32::MAX),
			});
		}
	};
	let name = card.title.clone();
	let button = "rounded-lg bg-slate-700 px-2 py-1 text-sm";

	view! {
		<View class="gap-2 rounded-lg bg-slate-800 p-3">
			<Text class="text-base">{card.title.clone()}</Text>
			<View class="flex-row gap-1">
				{(column > 0).then(|| {
					view! {
						<Button
							class=button
							accessibility_label=format!("Move {name} to {}", title(COLUMNS[column - 1]))
							on:press=to(column - 1, usize::MAX)
						>
							"←"
						</Button>
					}
				})}
				{(at > 0).then(|| {
					view! {
						<Button
							class=button
							accessibility_label=format!("Move {name} up")
							on:press=to(column, at - 1)
						>
							"↑"
						</Button>
					}
				})}
				{(at + 1 < count).then(|| {
					view! {
						<Button
							class=button
							accessibility_label=format!("Move {name} down")
							on:press=to(column, at + 1)
						>
							"↓"
						</Button>
					}
				})}
				{(column + 1 < COLUMNS.len()).then(|| {
					view! {
						<Button
							class=button
							accessibility_label=format!("Move {name} to {}", title(COLUMNS[column + 1]))
							on:press=to(column + 1, usize::MAX)
						>
							"→"
						</Button>
					}
				})}
			</View>
		</View>
	}
}
