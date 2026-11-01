//! A channel's messages, who's typing, and the composer.
//!
//! A message you send shows at once, greyed, and turns solid when the live
//! list holds the server's row with the same key: you read your own write,
//! and nothing flickers back. If it fails, it stays greyed with **Retry**,
//! which sends the same key again, so it's stored once however many times
//! it reaches the server.

use leptoskit::leptos::prelude::Get;
use leptoskit::leptos::prelude::Set;
use leptoskit::prelude::*;
use leptoskit::view;

use crate::huddle::Huddle;
use crate::huddle::Pending;
use crate::ui::Avatar;

#[component]
pub fn Panel(huddle: &Huddle) -> Element {
	let huddle = *huddle;
	let messages = huddle.messages.data().unwrap_or_default();
	let pending = huddle.pending_here(&messages);
	let channel = huddle
		.open_channel()
		.map(|card| card.name)
		.unwrap_or_default();
	let draft = huddle.draft.get();
	let typing = huddle.room.typing_line();
	let empty = messages.is_empty() && pending.is_empty();
	let rows: Fragment = messages
		.into_iter()
		.map(|message| {
			view! {
				<View class="flex-row gap-3 items-start">
					{view! { <Avatar name=&message.author /> }}
					<View class="flex-1 min-w-0 gap-0.5">
						<Text class="text-sm font-semibold">{message.author}</Text>
						<Text class="text-base">{message.body}</Text>
					</View>
				</View>
			}
		})
		.collect();
	let waiting: Fragment = pending
		.into_iter()
		.map(|message| view! { <PendingRow huddle=&huddle message=message /> })
		.collect();

	view! {
		<View class="flex-1 gap-3">
			<ScrollView class="flex-1 min-h-80 rounded-xl bg-slate-900 p-4">
				<View class="gap-4">
					{empty.then(|| {
						view! { <Text class="text-slate-400">"No messages yet. Say hello."</Text> }
					})}
					{rows} {waiting}
				</View>
			</ScrollView>
			<Status>
				<Text class="text-xs text-slate-400 h-4">{typing.unwrap_or_default()}</Text>
			</Status>
			<View class="flex-row gap-2">
				<TextInput
					class="flex-1 rounded-lg bg-slate-800 px-3 py-2 text-base"
					value=draft.clone()
					placeholder=format!("Message #{channel}")
					accessibility_label=format!("Message #{channel}")
					test_id="composer"
					on:change=move |text: &str| {
						huddle.draft.set(text.to_owned());
						if text.is_empty() {
							huddle.room.stopped_typing();
						} else {
							huddle.room.typing();
						}
					}
				/>
				<Button
					class="rounded-lg bg-indigo-500 px-4 py-2 font-semibold"
					enabled=!draft.trim().is_empty()
					test_id="send"
					on:press=move || huddle.send()
				>
					"Send"
				</Button>
			</View>
		</View>
	}
}

/// A message on its way, greyed: *Sending…*, or *Not sent* with Retry.
#[component]
fn PendingRow(huddle: &Huddle, message: Pending) -> Element {
	let huddle = *huddle;
	let key = message.client_key.clone();
	let state = if message.failed {
		"Not sent"
	} else {
		"Sending…"
	};

	view! {
		<View class="flex-row gap-3 items-start opacity-50" aria_busy=!message.failed>
			<Avatar name=&huddle
				.me
				.data()
				.flatten()
				.map(|me| me.name)
				.unwrap_or_default() />
			<View class="flex-1 min-w-0 gap-0.5">
				<Text class="text-sm font-semibold">{state}</Text>
				<Text class="text-base">{message.body}</Text>
			</View>
			{message.failed.then(|| {
				view! {
					<Button
						class="rounded-lg bg-slate-700 px-3 py-1 text-sm"
						accessibility_label="Send this message again"
						on:press=move || huddle.resend(&key)
					>
						"Retry"
					</Button>
				}
			})}
		</View>
	}
}
