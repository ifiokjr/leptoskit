//! The bell: `use_inbox`'s notices, newest first, on every device the
//! person has open at once.
//!
//! A notice can carry an action, which names a mutation and its arguments.
//! A failed tip's notice carries **Retry**, which runs `retry_tip` as the
//! person tapping it.

use huddle_api::RetryTip;
use leptoskit::leptos::prelude::WithValue;
use leptoskit::prelude::*;
use leptoskit::view;
use leptoskit_rpc::InboxNotice;

use crate::huddle::Huddle;

#[component]
pub fn Panel(huddle: &Huddle) -> Element {
	let huddle = *huddle;
	let notices = huddle
		.inbox
		.with_value(|inbox| inbox.notices.data())
		.map(|inbox| inbox.notices)
		.unwrap_or_default();
	let empty = notices.is_empty();
	let rows: Fragment = notices
		.into_iter()
		.map(|notice| view! { <Row huddle=&huddle notice=&notice /> })
		.collect();

	view! {
		<View class="gap-2 p-4 bg-slate-900" accessibility_label="Notices">
			<View class="flex-row items-center gap-2">
				<Text class="flex-1 text-sm font-semibold">"Notices"</Text>
				<Button
					class="rounded-lg bg-slate-700 px-3 py-1 text-sm"
					on:press=move || {
						huddle
							.inbox
							.with_value(leptoskit_rpc::client::InboxHandle::mark_all_read);
					}
				>
					"Mark all read"
				</Button>
			</View>
			{empty.then(|| {
				view! {
					<Text class="text-sm text-slate-400">
						"Nothing yet. Invites, reminders, and tips report here."
					</Text>
				}
			})}
			{rows}
		</View>
	}
}

#[component]
fn Row(huddle: &Huddle, notice: &InboxNotice) -> Element {
	let huddle = *huddle;
	let id = notice.id.clone();
	let action = notice.notice.action.clone().and_then(|action| {
		let retry = action.decode::<RetryTip>()?;
		let label = action.label;
		let id = id.clone();
		Some(view! {
			<Button
				class="rounded-lg bg-indigo-500 px-3 py-1 text-sm font-semibold"
				on:press=move || {
					huddle.inbox.with_value(|inbox| inbox.mark_read(id.clone()));
					huddle.run(retry.clone());
				}
			>
				{label}
			</Button>
		})
	});
	let weight = if notice.read {
		"text-sm"
	} else {
		"text-sm font-bold"
	};

	view! {
		<View class="flex-row gap-2 items-start rounded-lg bg-slate-800 p-3">
			<View class="flex-1 gap-0.5">
				<Text class=weight>{notice.notice.title.clone()}</Text>
				{(!notice.notice.body.is_empty()).then(|| {
					view! {
						<Text class="text-sm text-slate-300">{notice.notice.body.clone()}</Text>
					}
				})}
			</View>
			{action}
			{(!notice.read).then(|| {
				view! {
					<Button
						class="rounded-lg bg-slate-700 px-3 py-1 text-sm"
						accessibility_label=format!("Mark “{}” read", notice.notice.title)
						on:press=move || huddle.inbox.with_value(|inbox| inbox.mark_read(id.clone()))
					>
						"Read"
					</Button>
				}
			})}
		</View>
	}
}
