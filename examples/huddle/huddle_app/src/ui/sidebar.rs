//! The channels you're in, with what's unread, and starting a new one.
//!
//! The list is the live `channels` query. A private channel appears here
//! the moment its creator adds you, and vanishes when they take you out,
//! because the table's policy, which asks about memberships, decides what
//! the query returns.

use leptoskit::leptos::prelude::Get;
use leptoskit::leptos::prelude::GetUntracked;
use leptoskit::leptos::prelude::Set;
use leptoskit::prelude::*;
use leptoskit::view;

use crate::huddle::Huddle;

pub fn panel(huddle: &Huddle) -> Element {
	let huddle = *huddle;
	let open = huddle.selected.get();
	let channels = huddle.channels.data().unwrap_or_default();
	let rows: Fragment = channels
		.into_iter()
		.map(|card| {
			let id = card.id;
			let selected = id == open;
			let label = match card.unread {
				0 => format!("#{}", card.name),
				unread => format!("#{} ({unread})", card.name),
			};
			let spoken = match card.unread {
				0 => format!("{}, channel", card.name),
				1 => format!("{}, channel, 1 unread", card.name),
				unread => format!("{}, channel, {unread} unread", card.name),
			};

			view! {
				<Button
					class={if selected {
						"rounded-lg bg-indigo-500 px-3 py-2 text-sm font-semibold text-start"
					} else if card.unread > 0 {
						"rounded-lg bg-slate-800 px-3 py-2 text-sm font-bold text-start"
					} else {
						"rounded-lg bg-slate-900 px-3 py-2 text-sm text-start text-slate-300"
					}}
					accessibility_label=spoken
					aria_pressed=selected
					test_id=format!("channel-{}", card.name)
					on:press=move || huddle.selected.set(id)
				>
					{label}
				</Button>
			}
		})
		.collect();
	let name = huddle.new_channel.get();

	view! {
		<View class="gap-2 p-4 bg-slate-900 lg:w-64">
			<Text class="text-xs font-semibold text-slate-400">"CHANNELS"</Text>
			<View class="gap-1 flex-row flex-wrap lg:flex-col">{rows}</View>
			<View class="gap-2 pt-4">
				<TextInput
					class="rounded-lg bg-slate-800 px-3 py-2 text-sm"
					value=name.clone()
					placeholder="new-channel"
					accessibility_label="New channel name"
					on:change=move |text: &str| huddle.new_channel.set(text.to_owned())
				/>
				<View class="flex-row gap-2">
					<Button
						class="flex-1 rounded-lg bg-slate-700 px-3 py-2 text-sm font-semibold"
						enabled=!name.trim().is_empty()
						on:press=move || create(&huddle, false)
					>
						"Create"
					</Button>
					<Button
						class="flex-1 rounded-lg bg-slate-700 px-3 py-2 text-sm font-semibold"
						enabled=!name.trim().is_empty()
						accessibility_label="Create a private channel"
						on:press=move || create(&huddle, true)
					>
						"Create private"
					</Button>
				</View>
			</View>
		</View>
	}
}

fn create(huddle: &Huddle, private: bool) {
	let huddle = *huddle;
	let name = huddle.new_channel.get_untracked();
	huddle.new_channel.set(String::new());
	huddle.run(huddle_api::CreateChannel {
		name,
		private,
		members: Vec::new(),
	});
}
