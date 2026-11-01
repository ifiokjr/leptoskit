//! The guest's agent against a fake host bridge: what it sends, when, and
//! how the hub's commands come back. The test answers each call by hand and
//! polls the agent, as the bridge's waker would.

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

use leptoskit_bridge::Error;
use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::Value;
use leptoskit_plugin_devtools::Hello;
use leptoskit_plugin_devtools::Report;
use leptoskit_router::Router;
use reactive_graph::owner::Owner;
use reactive_graph::signal::RwSignal;
use reactive_graph::traits::GetUntracked;
use reactive_graph::traits::Set;
use serde_json::json;

use super::Agent;
use super::DEBOUNCE_MS;
use super::Pending;
use super::Transport;
use super::Wake;
use crate::expose;
use crate::registry;
use crate::tests::Page;
use crate::track_router;

type Answer<T> = Rc<RefCell<Option<Result<T, Error>>>>;

/// A call the test answers.
struct Call<T>(Answer<T>);

impl<T> Future for Call<T> {
	type Output = Result<T, Error>;

	fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
		self.0
			.borrow_mut()
			.take()
			.map_or(Poll::Pending, Poll::Ready)
	}
}

fn call<T: 'static>(waiting: &RefCell<VecDeque<Answer<T>>>) -> Pending<T> {
	let answer = Answer::default();
	waiting.borrow_mut().push_back(Rc::clone(&answer));
	Box::pin(Call(answer))
}

/// The host side of the bridge.
#[derive(Default)]
struct Host {
	hellos: RefCell<Vec<Hello>>,
	/// Every report, decoded, in the order sent.
	reports: RefCell<Vec<Report>>,
	waiting_reports: RefCell<VecDeque<Answer<()>>>,
	waiting_commands: RefCell<VecDeque<Answer<Vec<Command>>>>,
	waiting_waits: RefCell<VecDeque<Answer<()>>>,
	/// Whether the host refuses waits.
	no_waits: Cell<bool>,
	renders: Cell<usize>,
}

impl Transport for Rc<Host> {
	fn hello(&self, hello: &Hello) -> Pending<()> {
		self.hellos.borrow_mut().push(hello.clone());
		Box::pin(async { Ok(()) })
	}

	fn report(&self, payload: &[u8]) -> Pending<()> {
		self.reports
			.borrow_mut()
			.push(Report::decode(payload).expect("a valid report"));
		call(&self.waiting_reports)
	}

	fn commands(&self) -> Pending<Vec<Command>> {
		call(&self.waiting_commands)
	}

	fn wait(&self, ms: u32) -> Pending<()> {
		assert_eq!(ms, DEBOUNCE_MS);

		if self.no_waits.get() {
			return Box::pin(async { Err(Error::Busy) });
		}

		call(&self.waiting_waits)
	}

	fn render(&self) {
		self.renders.set(self.renders.get() + 1);
	}
}

struct Guest {
	host: Rc<Host>,
	agent: Rc<Agent<Rc<Host>>>,
	_owner: Owner,
}

impl Guest {
	/// An agent over a fresh registry, under a root owner for the test's
	/// signals.
	///
	/// Create signals after this: once workspace feature unification turns
	/// on `reactive_graph`'s `sandboxed-arenas`, as `cargo test --workspace`
	/// and nextest do, a signal created before any owner is set has no arena
	/// to live in.
	fn new(host: Host) -> Self {
		registry::reset();
		let owner = Owner::new();
		owner.set();
		let host = Rc::new(host);
		let agent = Rc::new(Agent::new(Rc::clone(&host), Waker::noop().clone()));
		registry::install(Rc::new(Wake(Rc::clone(&agent))));
		Self {
			host,
			agent,
			_owner: owner,
		}
	}

	/// Runs the app's `hooks`, then starts the agent.
	fn start(&self, hooks: impl FnOnce()) {
		hooks();
		self.agent.start();
	}

	/// The host's clock passes the debounce, if the agent is waiting it out.
	fn settle(&self) {
		let waiting = self.host.waiting_waits.borrow_mut().pop_front();

		if let Some(answer) = waiting {
			*answer.borrow_mut() = Some(Ok(()));
		}

		self.agent.poll();
	}

	/// Answers the oldest waiting report.
	fn answer_report(&self, result: Result<(), Error>) {
		let waiting = self.host.waiting_reports.borrow_mut().pop_front();
		*waiting.expect("a report waits").borrow_mut() = Some(result);
		self.agent.poll();
	}

	/// The hub sends `commands`.
	fn send(&self, commands: Vec<Command>) {
		let waiting = self.host.waiting_commands.borrow_mut().pop_front();
		*waiting.expect("a commands call waits").borrow_mut() = Some(Ok(commands));
		self.agent.poll();
	}

	fn reports(&self) -> Vec<Report> {
		self.host.reports.borrow().clone()
	}

	fn last_report(&self) -> Report {
		self.host
			.reports
			.borrow()
			.last()
			.cloned()
			.expect("a report")
	}
}

fn count(value: usize) -> Event {
	Event::StateChanged {
		name: "count".into(),
		value: Value::from_json("usize", json!(value)),
	}
}

fn set_count(value: usize) -> Command {
	Command::SetState {
		name: "count".into(),
		value: Value::from_json("usize", json!(value)),
	}
}

#[test]
fn a_guest_says_what_it_feeds_then_reports_every_value_after_the_debounce() {
	let guest = Guest::new(Host::default());
	let signal = RwSignal::new(1_usize);
	guest.start(|| expose("count", signal));

	assert_eq!(
		*guest.host.hellos.borrow(),
		[Hello::new(registry::capabilities())]
	);
	assert_eq!(guest.host.waiting_commands.borrow().len(), 1);
	assert!(guest.reports().is_empty(), "nothing before the debounce");
	assert_eq!(guest.host.waiting_waits.borrow().len(), 1);

	guest.settle();
	assert_eq!(
		guest.reports(),
		[Report {
			reset: true,
			events: vec![count(1)],
		}]
	);
	guest.answer_report(Ok(()));
	assert!(
		guest.host.waiting_waits.borrow().is_empty(),
		"an idle agent waits for nothing"
	);
}

#[test]
fn a_burst_of_changes_costs_one_report_of_the_latest_value() {
	let guest = Guest::new(Host::default());
	let signal = RwSignal::new(1_usize);
	guest.start(|| expose("count", signal));
	guest.settle();
	guest.answer_report(Ok(()));

	signal.set(2);
	signal.set(3);
	signal.set(4);
	assert_eq!(
		guest.host.waiting_waits.borrow().len(),
		1,
		"one wait for the burst"
	);
	guest.settle();
	assert_eq!(
		guest.reports()[1..],
		[Report {
			reset: false,
			events: vec![count(4)],
		}]
	);
}

#[test]
fn one_report_posts_at_a_time() {
	let guest = Guest::new(Host::default());
	let signal = RwSignal::new(1_usize);
	guest.start(|| expose("count", signal));
	guest.settle();

	// The first report hasn't been answered: the next change waits for it.
	signal.set(2);
	guest.settle();
	assert_eq!(guest.reports().len(), 1);
	guest.answer_report(Ok(()));
	guest.settle();
	assert_eq!(guest.last_report().events, [count(2)]);
}

#[test]
fn set_state_from_the_hub_sets_the_signal_renders_and_reports_it() {
	let guest = Guest::new(Host::default());
	let signal = RwSignal::new(1_usize);
	guest.start(|| expose("count", signal));
	guest.settle();
	guest.answer_report(Ok(()));

	guest.send(vec![set_count(7)]);
	assert_eq!(signal.get_untracked(), 7);
	assert_eq!(guest.host.renders.get(), 1);
	assert_eq!(
		guest.host.waiting_commands.borrow().len(),
		1,
		"the next commands call is open"
	);
	guest.settle();
	assert_eq!(guest.last_report().events, [count(7)]);

	// A command for state the app doesn't expose changes nothing, so nothing
	// renders.
	guest.send(vec![Command::SetState {
		name: "missing".into(),
		value: Value::from_json("usize", json!(1)),
	}]);
	assert_eq!(guest.host.renders.get(), 1);
}

#[test]
fn navigate_from_the_hub_moves_the_tracked_router() {
	let router = Router::<Page>::new(Page::Home);
	let tracked = router.clone();
	let guest = Guest::new(Host::default());
	guest.start(move || track_router(&tracked));
	guest.settle();
	assert_eq!(
		guest.last_report().events,
		[Event::RouteChanged { route: "/".into() }]
	);
	guest.answer_report(Ok(()));

	guest.send(vec![Command::Navigate {
		href: "/settings".into(),
	}]);
	assert_eq!(
		reactive_graph::graph::untrack(|| router.current()),
		Page::Settings
	);
	guest.settle();
	assert_eq!(
		guest.last_report(),
		Report {
			reset: false,
			events: vec![Event::RouteChanged {
				route: "/settings".into(),
			}],
		}
	);
}

#[test]
fn a_snapshot_request_resends_every_value_even_while_a_report_posts() {
	let guest = Guest::new(Host::default());
	let signal = RwSignal::new(1_usize);
	guest.start(|| expose("count", signal));
	guest.settle();

	// The request arrives while the first report is still posting.
	guest.send(vec![Command::RequestSnapshot]);
	guest.answer_report(Ok(()));
	guest.settle();
	assert_eq!(
		guest.last_report(),
		Report {
			reset: true,
			events: vec![count(1)],
		}
	);
}

#[test]
fn a_refused_report_resends_every_value() {
	let guest = Guest::new(Host::default());
	let signal = RwSignal::new(1_usize);
	guest.start(|| expose("count", signal));
	guest.settle();
	guest.answer_report(Ok(()));
	signal.set(2);
	guest.settle();
	guest.answer_report(Err(Error::Busy));

	guest.settle();
	assert_eq!(
		guest.last_report(),
		Report {
			reset: true,
			events: vec![count(2)],
		}
	);
}

#[test]
fn a_wait_the_host_refuses_flushes_at_once() {
	let host = Host::default();
	host.no_waits.set(true);
	let guest = Guest::new(host);
	let signal = RwSignal::new(1_usize);
	guest.start(|| expose("count", signal));
	assert_eq!(guest.last_report().events, [count(1)]);
}

#[test]
fn a_closed_host_ends_the_commands_call() {
	let guest = Guest::new(Host::default());
	guest.start(|| {});
	let waiting = guest.host.waiting_commands.borrow_mut().pop_front();
	*waiting.unwrap().borrow_mut() = Some(Err(Error::Cancelled));
	guest.agent.poll();
	assert!(
		guest.host.waiting_commands.borrow().is_empty(),
		"no new call to a host that closed"
	);
}
