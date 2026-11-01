//! The signed-in screens, rendered headlessly with the real layout engine
//! at a laptop's width and a phone's, from data put straight into the query
//! cache: every tab, a message on its way, a failed tip with Retry, and the
//! bell. Rendering checks every Tailwind class each screen uses.

use std::cell::RefCell;

use futures::executor::LocalPool;
use futures::executor::LocalSpawner;
use futures::task::LocalSpawnExt as _;
use huddle_api::ChannelCard;
use huddle_api::InviteView;
use huddle_api::MessageView;
use huddle_api::Person;
use huddle_api::TaskCard;
use huddle_api::TipView;
use huddle_app::Shell;
use huddle_app::auth::SignIn;
use huddle_app::huddle::Huddle;
use huddle_app::huddle::Pending;
use huddle_app::huddle::Tab;
use leptoskit::db::Id;
use leptoskit::leptos::prelude::*;
use leptoskit::query::QueryClient;
use leptoskit::query::use_query_client;
use leptoskit_rpc::client::Args;
use leptoskit_rpc::client::query_scope;
use leptoskit_testing::Finder;
use leptoskit_testing::Queries;
use leptoskit_testing::WidgetTester;

thread_local! {
	static POOL: RefCell<LocalPool> = RefCell::new(LocalPool::new());
	static SPAWNER: LocalSpawner = POOL.with_borrow(LocalPool::spawner);
}

/// Each test thread's own task pool, run by [`settle`], as the native app
/// runs its own. Nothing reaches a server: every call fails, and the screens
/// show what the test put in the cache.
struct Pool;

impl any_spawner::CustomExecutor for Pool {
	fn spawn(&self, task: any_spawner::PinnedFuture<()>) {
		self.spawn_local(task);
	}

	fn spawn_local(&self, task: any_spawner::PinnedLocalFuture<()>) {
		// While the thread ends, the pool is gone and the task is dropped.
		let _ = SPAWNER.try_with(|spawner| spawner.spawn_local(task));
	}

	fn poll_local(&self) {}
}

/// Runs every task and effect that can make progress.
fn settle() {
	POOL.with_borrow_mut(LocalPool::run_until_stalled);
}

/// Ends the test without dropping the tasks still waiting, such as the
/// live queries' subscriptions: they would run their cleanup after the
/// thread's reactive runtime is gone.
fn finish(owner: Owner) {
	std::mem::forget(owner);
	std::mem::forget(POOL.with_borrow_mut(std::mem::take));
}

fn id<T>(n: u8) -> Id<T> {
	Id::parse(&format!("0192f3c4-0000-7000-8000-0000000000{n:02}")).expect("an ID")
}

fn seed<Q>(client: &QueryClient, args: Q, value: Q::Output)
where
	Q: leptoskit_rpc::Operation,
	Q::Output: Clone,
{
	client.set_query_data(&query_scope::<Q>(), Args::new(args), value);
}

fn huddle() -> (Owner, Shell) {
	SPAWNER.with(|_| ());
	let _ = any_spawner::Executor::init_local_custom_executor(Pool);
	let owner = Owner::new();
	let shell = owner.with(|| {
		QueryClient::new().provide();
		let huddle = Huddle::new();
		let client = use_query_client();
		let general = id(1);
		let ada = Person {
			id: id(10),
			name: "Ada".into(),
			is_admin: true,
		};
		let grace = Person {
			id: id(11),
			name: "Grace Hopper".into(),
			is_admin: false,
		};

		seed(&client, huddle_api::Whoami {}, Some(ada.clone()));
		seed(
			&client,
			huddle_api::People {},
			vec![ada.clone(), grace.clone()],
		);
		seed(
			&client,
			huddle_api::Channels {},
			vec![
				ChannelCard {
					id: general,
					name: "general".into(),
					private: false,
					owned: true,
					unread: 0,
					summary: Some("2 messages from 2 people in #general.".into()),
				},
				ChannelCard {
					id: id(2),
					name: "launch".into(),
					private: true,
					owned: false,
					unread: 3,
					summary: None,
				},
			],
		);
		seed(
			&client,
			huddle_api::Messages {
				channel: general,
				limit: huddle_api::PAGE,
				before: None,
			},
			vec![MessageView {
				id: id(20),
				author_id: grace.id,
				author: "Grace Hopper".into(),
				body: "Ship it on Thursday?".into(),
				client_key: "grace-1".into(),
				sent_at: "2026-11-01T10:00:00Z".into(),
			}],
		);
		seed(
			&client,
			huddle_api::Members { channel: general },
			vec![ada.clone(), grace.clone()],
		);
		seed(
			&client,
			huddle_api::Board { channel: general },
			vec![
				TaskCard {
					id: id(30),
					title: "Write the launch post".into(),
					column: "todo".into(),
					position: 0,
				},
				TaskCard {
					id: id(31),
					title: "Fix sign-in".into(),
					column: "doing".into(),
					position: 0,
				},
			],
		);
		seed(
			&client,
			huddle_api::Invites { channel: general },
			vec![InviteView {
				id: id(40),
				email: "lin@huddle.dev".into(),
				status: "sent".into(),
			}],
		);
		seed(
			&client,
			huddle_api::Tips {},
			vec![TipView {
				id: id(50),
				from_id: ada.id,
				from: "Ada".into(),
				to_id: grace.id,
				to: "Grace Hopper".into(),
				amount: 5,
				status: "failed".into(),
				reason: Some("The settlement service declined the tip (simulated)".into()),
				receipt: None,
			}],
		);

		huddle.selected.set(general);
		huddle.pending.set(vec![Pending {
			channel: general,
			client_key: "ada-1".into(),
			body: "Thursday works".into(),
			failed: false,
		}]);

		Shell {
			sign_in: SignIn::new(),
			huddle: RwSignal::new(Some(huddle)),
		}
	});
	settle();
	(owner, shell)
}

fn tester(owner: &Owner, shell: &Shell, width: f32, height: f32) -> WidgetTester {
	let owner = owner.clone();
	let shell = *shell;
	WidgetTester::new(
		move || {
			owner.with(|| {
				let huddle = shell.huddle.get_untracked().expect("signed in");
				huddle_app::ui::screen(&shell, &huddle)
			})
		},
		width,
		height,
	)
	.expect("the screen mounts")
}

#[test]
fn every_tab_renders_on_a_laptop_and_a_phone() {
	let (owner, shell) = huddle();
	let huddle = shell.huddle.get_untracked().expect("signed in");

	// Tall enough that every section is on screen: finders match what a
	// person could see.
	for (width, height) in [(1280.0, 2400.0), (390.0, 3600.0)] {
		huddle.tab.set(Tab::Chat);
		let mut screen = tester(&owner, &shell, width, height);

		assert_eq!(screen.get_all(&Finder::text("#general")).unwrap().len(), 2);
		screen.get(&Finder::text("Ship it on Thursday?")).unwrap();
		screen
			.get(&Finder::label("launch, channel, 3 unread"))
			.unwrap();
		// The message on its way shows at once, greyed, until the live list
		// has it.
		screen.get(&Finder::text("Thursday works")).unwrap();
		screen.get(&Finder::text("Sending…")).unwrap();
		screen
			.get(&Finder::text("2 messages from 2 people in #general."))
			.unwrap();

		huddle.tab.set(Tab::Board);
		settle();
		screen.pump();
		screen.get(&Finder::text("To do · 1")).unwrap();
		screen
			.get(&Finder::label("Move Write the launch post to Doing"))
			.unwrap();
		screen
			.get(&Finder::label("Move Fix sign-in to To do"))
			.unwrap();

		huddle.tab.set(Tab::Activity);
		settle();
		screen.pump();
		screen.get(&Finder::text("Email sent")).unwrap();
		screen
			.get(&Finder::text(
				"Failed: The settlement service declined the tip (simulated)",
			))
			.unwrap();
		screen
			.get(&Finder::label("Retry the tip to Grace Hopper"))
			.unwrap();
		screen
			.get(&Finder::label("Sign Grace Hopper out everywhere"))
			.unwrap();

		huddle.show_inbox.set(true);
		settle();
		screen.pump();
		screen
			.get(&Finder::text(
				"Nothing yet. Invites, reminders, and tips report here.",
			))
			.unwrap();
		huddle.show_inbox.set(false);
	}

	finish(owner);
}

#[test]
fn a_message_that_failed_offers_retry() {
	let (owner, shell) = huddle();
	let huddle = shell.huddle.get_untracked().expect("signed in");
	huddle.pending.update(|pending| pending[0].failed = true);
	settle();
	let screen = tester(&owner, &shell, 1280.0, 900.0);

	screen.get(&Finder::text("Not sent")).unwrap();
	screen
		.get(&Finder::label("Send this message again"))
		.unwrap();
	drop(screen);
	finish(owner);
}
