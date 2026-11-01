//! The signed-in screens, as `view!` markup over [`Huddle`]'s queries.
//!
//! Wide screens put the sidebar beside the channel; phones stack them.
//! Every control has a label a screen reader says, and the status lines
//! (typing, sending, a job's progress) are live regions.

mod activity;
mod bell;
mod board;
mod chat;
mod sidebar;

use leptoskit::leptos::prelude::Get;
use leptoskit::leptos::prelude::GetUntracked;
use leptoskit::leptos::prelude::Set;
use leptoskit::leptos::prelude::Update;
use leptoskit::leptos::prelude::WithValue;
use leptoskit::prelude::*;
use leptoskit::view;

use crate::Shell;
use crate::huddle::Huddle;
use crate::huddle::Tab;

/// A small round badge with someone's initials.
pub fn avatar(name: &str) -> Element {
	let initials: String = name
		.split_whitespace()
		.filter_map(|word| word.chars().next())
		.take(2)
		.flat_map(char::to_uppercase)
		.collect();

	view! {
		<View
			class="w-8 h-8 rounded-[16px] bg-indigo-500 items-center justify-center"
			accessibility_label=name.to_owned()
		>
			<Text class="text-xs font-semibold text-white">{initials}</Text>
		</View>
	}
}

/// The whole signed-in interface.
pub fn screen(shell: &Shell, huddle: &Huddle) -> Element {
	let huddle = *huddle;
	let shell = *shell;
	let body = match huddle.tab.get() {
		Tab::Chat => chat::panel(&huddle),
		Tab::Board => board::panel(&huddle),
		Tab::Activity => activity::panel(&huddle),
	};

	view! {
		<View class="min-h-screen bg-slate-950 text-white">
			{top_bar(&shell, &huddle)} {huddle.show_inbox.get().then(|| bell::panel(&huddle))}
			<View class="flex-1 flex-col lg:flex-row">
				{sidebar::panel(&huddle)}
				<View class="flex-1 min-w-0 p-4 gap-3">{channel_header(&huddle)} {body}</View>
			</View>
		</View>
	}
}

/// The app's name, who's in the open channel right now, the bell, and the
/// signed-in person.
fn top_bar(shell: &Shell, huddle: &Huddle) -> Element {
	let huddle = *huddle;
	let shell = *shell;
	let here = huddle.room.here();
	let count = here.len();
	let unread = huddle
		.inbox
		.with_value(leptoskit_rpc::client::InboxHandle::unread);
	let bell = if unread == 0 {
		"Notices".to_owned()
	} else {
		format!("Notices ({unread})")
	};
	let me = huddle
		.me
		.data()
		.flatten()
		.map(|me| me.name)
		.unwrap_or_default();
	let sign_in = shell.sign_in;

	view! {
		<View class="flex-row items-center gap-3 px-4 py-3 bg-slate-900">
			<Text class="text-xl font-bold">"Huddle"</Text>
			<View
				class="flex-row items-center gap-1 flex-1"
				accessibility_label=format!("{count} here now")
			>
				{here
					.iter()
					.map(|person| avatar(&person.name))
					.collect::<Fragment>()}
			</View>
			<Button
				class="rounded-lg bg-slate-700 px-3 py-2 text-sm font-semibold"
				accessibility_label=if unread == 0 {
					"Notices, none unread".to_owned()
				} else {
					format!("Notices, {unread} unread")
				}
				aria_expanded=huddle.show_inbox.get()
				test_id="bell"
				on:press=move || huddle.show_inbox.update(|open| *open = !*open)
			>
				{bell}
			</Button>
			<Text class="text-sm text-slate-300">{me}</Text>
			<Button
				class="rounded-lg bg-slate-700 px-3 py-2 text-sm font-semibold"
				on:press=move || sign_in.sign_out()
			>
				"Sign out"
			</Button>
		</View>
	}
}

/// `#general`, its pinned summary, and the tabs.
fn channel_header(huddle: &Huddle) -> Element {
	let huddle = *huddle;
	let Some(channel) = huddle.open_channel() else {
		return view! {
			<Status>
				<Text class="text-slate-300">"Loading your channels…"</Text>
			</Status>
		};
	};
	let tab = huddle.tab.get();
	let tab_button = move |label: &'static str, which: Tab| {
		view! {
			<Button
				class=if tab == which {
					"rounded-lg bg-indigo-500 px-3 py-2 text-sm font-semibold"
				} else {
					"rounded-lg bg-slate-800 px-3 py-2 text-sm font-semibold"
				}
				aria_pressed=tab == which
				on:press=move || huddle.tab.set(which)
			>
				{label}
			</Button>
		}
	};
	let summarizing = huddle.summarize.is_pending();

	view! {
		<View class="gap-2">
			<View class="flex-row items-center gap-3 flex-wrap">
				<Text class="text-2xl font-bold">{format!("#{}", channel.name)}</Text>
				{channel.private.then(|| {
					view! {
						<Text class="text-xs rounded-lg bg-slate-700 px-2 py-0.5">"Private"</Text>
					}
				})}
				<View class="flex-1" />
				{tab_button("Chat", Tab::Chat)}
				{tab_button("Board", Tab::Board)}
				{tab_button("Activity", Tab::Activity)}
			</View>
			<View class="flex-row items-center gap-2 flex-wrap">
				<Button
					class="rounded-lg bg-slate-800 px-3 py-2 text-sm"
					enabled=!summarizing
					aria_busy=summarizing
					on:press=move || {
						huddle.summarize.mutate(huddle_api::SummarizeChannel {
							channel: huddle.selected.get_untracked(),
						});
					}
				>
					{if summarizing {
						"Summarizing…"
					} else {
						"Summarize this channel"
					}}
				</Button>
				{channel.summary.map(|summary| {
					view! {
						<Text
							class="flex-1 text-sm text-slate-300"
							accessibility_label="Pinned summary"
						>
							{summary}
						</Text>
					}
				})}
			</View>
			{huddle.problem.get().map(|problem| {
				view! {
					<Alert>
						<Text class="text-sm text-rose-300">{problem}</Text>
					</Alert>
				}
			})}
		</View>
	}
}
