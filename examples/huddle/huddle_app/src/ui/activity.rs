//! Slow work that reports back: invitations, reminders, tips, and who's in
//! the channel.
//!
//! Each button runs a mutation that writes a row and enqueues a job in the
//! same transaction, then returns. The job updates the row as it goes, and
//! the row is in a live table, so *Sending…* turns into *Email sent* and
//! *Pending* into *Confirmed* (or *Failed*, with **Retry**) on every screen
//! showing it, without a refresh.

use huddle_api::Person;
use huddle_api::TipView;
use leptoskit::leptos::prelude::Get;
use leptoskit::leptos::prelude::GetUntracked;
use leptoskit::leptos::prelude::Set;
use leptoskit::prelude::*;
use leptoskit::view;

use crate::huddle::Huddle;

const BUTTON: &str = "rounded-lg bg-slate-700 px-3 py-2 text-sm font-semibold";

fn section(title: &'static str, body: Element) -> Element {
	view! {
		<View class="gap-2 rounded-xl bg-slate-900 p-4" accessibility_label=title>
			<Text class="text-sm font-semibold text-slate-300">{title}</Text>
			{body}
		</View>
	}
}

pub fn panel(huddle: &Huddle) -> Element {
	let huddle = *huddle;
	view! {
		<ScrollView class="flex-1">
			<View class="gap-3">
				{section("Invite by email", invites(&huddle))}
				{section("Remind me", reminder(&huddle))} {section("Tip a teammate", tips(&huddle))}
				{section("People in this channel", people(&huddle))}
			</View>
		</ScrollView>
	}
}

fn invites(huddle: &Huddle) -> Element {
	let huddle = *huddle;
	let email = huddle.invite_email.get();
	let rows: Fragment = huddle
		.invites
		.data()
		.unwrap_or_default()
		.into_iter()
		.map(|invite| {
			let status = match invite.status.as_str() {
				"sending" => "Sending…",
				"sent" => "Email sent",
				_ => "Couldn't send",
			};
			view! {
				<View class="flex-row gap-2 items-center">
					<Text class="flex-1 text-sm">{invite.email}</Text>
					<Status>
						<Text class="text-sm text-slate-300">{status}</Text>
					</Status>
				</View>
			}
		})
		.collect();

	view! {
		<View class="gap-2">
			<View class="flex-row gap-2">
				<TextInput
					class="flex-1 rounded-lg bg-slate-800 px-3 py-2 text-base"
					value=email.clone()
					placeholder="name@example.com"
					accessibility_label="Email address to invite"
					on:change=move |text: &str| huddle.invite_email.set(text.to_owned())
				/>
				<Button
					class=BUTTON
					enabled=email.contains('@')
					on:press=move || {
						let email = huddle.invite_email.get_untracked();
						huddle.invite_email.set(String::new());
						huddle.run(huddle_api::Invite {
							channel: huddle.selected.get_untracked(),
							email,
						});
					}
				>
					"Invite"
				</Button>
			</View>
			{rows}
			<Text class="text-xs text-slate-400">
				"In development the email lands at /_leptoskit/mail on the server."
			</Text>
		</View>
	}
}

fn reminder(huddle: &Huddle) -> Element {
	let huddle = *huddle;
	let pending = huddle.reminder.is_pending();
	let set = huddle.reminder.data().is_some();
	let status = if pending {
		"Setting a reminder…"
	} else if set {
		"Reminder set: it arrives in Notices in 10 minutes, on every device."
	} else {
		"A notice about this channel, in 10 minutes."
	};

	view! {
		<View class="flex-row gap-2 items-center">
			<Button
				class=BUTTON
				enabled=!pending
				on:press=move || {
					huddle.reminder.mutate(huddle_api::RemindMe {
						channel: huddle.selected.get_untracked(),
						minutes: 10,
					});
				}
			>
				"Remind me in 10 minutes"
			</Button>
			<Status class="flex-1">
				<Text class="text-sm text-slate-300">{status}</Text>
			</Status>
		</View>
	}
}

fn tip_status(tip: &TipView) -> String {
	match tip.status.as_str() {
		"pending" => "Pending".to_owned(),
		"submitted" => "Submitted, waiting for confirmation".to_owned(),
		"confirmed" => "Confirmed".to_owned(),
		_ => format!("Failed: {}", tip.reason.clone().unwrap_or_default()),
	}
}

fn tips(huddle: &Huddle) -> Element {
	let huddle = *huddle;
	let me = huddle.my_id();
	let amount = huddle.tip_amount.get();
	let coins: i64 = amount.trim().parse().unwrap_or(0);
	let teammates: Fragment = huddle
		.members
		.data()
		.unwrap_or_default()
		.into_iter()
		.filter(|person| Some(person.id) != me)
		.map(|person| {
			let to = person.id;
			view! {
				<Button
					class=BUTTON
					enabled=(1..=1000).contains(&coins)
					accessibility_label=format!("Tip {} {coins} coins", person.name)
					on:press=move || huddle.run(huddle_api::SendTip { to, amount: coins })
				>
					{format!("Tip {}", person.name)}
				</Button>
			}
		})
		.collect();
	let rows: Fragment = huddle
		.tips
		.data()
		.unwrap_or_default()
		.into_iter()
		.map(|tip| {
			let sent = Some(tip.from_id) == me;
			let line = if sent {
				format!("{} coins to {}", tip.amount, tip.to)
			} else {
				format!("{} coins from {}", tip.amount, tip.from)
			};
			let id = tip.id;
			let retry = (sent && tip.status == "failed").then(|| {
				view! {
					<Button
						class=BUTTON
						accessibility_label=format!("Retry the tip to {}", tip.to)
						on:press=move || huddle.run(huddle_api::RetryTip { tip: id })
					>
						"Retry"
					</Button>
				}
			});
			view! {
				<View class="flex-row gap-2 items-center flex-wrap">
					<Text class="flex-1 text-sm">{line}</Text>
					<Status>
						<Text class="text-sm text-slate-300">{tip_status(&tip)}</Text>
					</Status>
					{retry}
				</View>
			}
		})
		.collect();

	view! {
		<View class="gap-2">
			<Text class="text-xs text-slate-400">
				"Coins are pretend, and settle through a simulated service inside the server: it takes a couple of seconds and declines some tips, so you can see each state."
			</Text>
			<View class="flex-row gap-2 items-center flex-wrap">
				<TextInput
					class="w-16 rounded-lg bg-slate-800 px-3 py-2 text-base"
					value=amount.clone()
					accessibility_label="Coins to tip"
					on:change=move |text: &str| huddle.tip_amount.set(text.to_owned())
				/>
				{teammates}
			</View>
			{rows}
		</View>
	}
}

fn people(huddle: &Huddle) -> Element {
	let huddle = *huddle;
	let owned = huddle.open_channel().is_some_and(|card| card.owned);
	let me = huddle.me.data().flatten();
	let admin = me.as_ref().is_some_and(|me| me.is_admin);
	let my_id = me.map(|me| me.id);
	let members = huddle.members.data().unwrap_or_default();
	let everyone = huddle.people.data().unwrap_or_default();
	let channel = huddle.selected.get();
	let rows: Fragment = everyone
		.into_iter()
		.filter(|person| owned || members.iter().any(|member| member.id == person.id))
		.map(|person: Person| {
			let user = person.id;
			let member = members.iter().any(|member| member.id == user);
			let mine = Some(user) == my_id;
			let membership = (owned && !mine).then(|| {
				if member {
					view! {
						<Button
							class=BUTTON
							accessibility_label=format!("Remove {} from this channel", person.name)
							on:press=move || huddle.run(huddle_api::RemoveMember { channel, user })
						>
							"Remove"
						</Button>
					}
				} else {
					view! {
						<Button
							class=BUTTON
							accessibility_label=format!("Add {} to this channel", person.name)
							on:press=move || huddle.run(huddle_api::AddMember { channel, user })
						>
							"Add"
						</Button>
					}
				}
			});
			let sign_out = (admin && !mine).then(|| {
				view! {
					<Button
						class="rounded-lg bg-rose-700 px-3 py-2 text-sm font-semibold"
						accessibility_label=format!("Sign {} out everywhere", person.name)
						on:press=move || {
							huddle
								.sign_out_everywhere
								.mutate(huddle_api::SignOutEverywhere { user });
						}
					>
						"Sign out everywhere"
					</Button>
				}
			});
			view! {
				<View class="flex-row gap-2 items-center flex-wrap">
					{crate::ui::avatar(&person.name)}
					<Text class="flex-1 text-sm">
						{if member {
							person.name.clone()
						} else {
							format!("{} (not in this channel)", person.name)
						}}
					</Text>
					{membership} {sign_out}
				</View>
			}
		})
		.collect();

	view! {
		<View class="gap-2">
			{owned.then(|| {
				view! {
					<Text class="text-xs text-slate-400">
						"You made this channel, so you decide who's in it. Someone you add sees it in their sidebar at once; someone you remove stops seeing it."
					</Text>
				}
			})}
			{rows}
		</View>
	}
}
