//! The hooks, the registry, and the outbox on the host: no browser and no
//! hub. A fake agent stands in for the web agent, which drains the same
//! registry.

use std::cell::Cell;
use std::rc::Rc;

use leptoskit_devtools_protocol::Capability;
use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::MatchKey;
use leptoskit_devtools_protocol::Node;
use leptoskit_devtools_protocol::NodeRef;
use leptoskit_devtools_protocol::Tree;
use leptoskit_devtools_protocol::Value;
use leptoskit_router::Href;
use leptoskit_router::NoMatch;
use leptoskit_router::Pattern;
use leptoskit_router::Routable;
use leptoskit_router::RouteUrl;
use leptoskit_router::Router;
use leptoskit_router::Segment;
use reactive_graph::graph::untrack;
use reactive_graph::owner::Owner;
use reactive_graph::signal::ArcRwSignal;
use reactive_graph::signal::RwSignal;
use reactive_graph::traits::GetUntracked;
use reactive_graph::traits::Set;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;

use crate::expose;
use crate::registry;
use crate::registry::Sink;
use crate::report::Outbox;
use crate::track_router;

/// Counts how often the hooks wake it.
#[derive(Default)]
struct FakeAgent {
	wakes: Cell<usize>,
}

impl Sink for FakeAgent {
	fn wake(&self) {
		self.wakes.set(self.wakes.get() + 1);
	}
}

/// A fresh registry with a fake agent, and a root owner for the test's
/// signals.
fn agent() -> (Rc<FakeAgent>, Owner) {
	registry::reset();
	let agent = Rc::new(FakeAgent::default());
	registry::install(agent.clone());
	let owner = Owner::new();
	owner.set();
	(agent, owner)
}

#[derive(Clone, Debug, PartialEq)]
enum Page {
	Home,
	Settings,
}

impl Routable for Page {
	fn from_url(url: &RouteUrl) -> Result<Self, NoMatch> {
		match url.segments() {
			[] => Ok(Self::Home),
			[settings] if settings == "settings" => Ok(Self::Settings),
			_ => Err(NoMatch),
		}
	}

	fn href(&self) -> Href {
		match self {
			Self::Home => Pattern::new(&[]).static_href(),
			Self::Settings => Pattern::new(&[Segment::Static("settings")]).static_href(),
		}
	}
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Draft {
	title: String,
	tags: Vec<String>,
}

fn route(route: &str) -> Event {
	Event::RouteChanged {
		route: route.into(),
	}
}

fn state(name: &str, type_name: &str, json: serde_json::Value) -> Event {
	Event::StateChanged {
		name: name.into(),
		value: Value::from_json(type_name, json),
	}
}

fn set_state(name: &str, json: serde_json::Value) -> Command {
	Command::SetState {
		name: name.into(),
		value: Value::from_json("usize", json),
	}
}

fn tree(kind: &str) -> Tree {
	Tree {
		root: NodeRef::Web(String::new()),
		nodes: vec![Node {
			id: NodeRef::Web(String::new()),
			key: MatchKey::new("root"),
			parent: None,
			kind: kind.into(),
			source: None,
			frame: None,
			classes: Vec::new(),
			text: None,
			children: Vec::new(),
			component: None,
			styles: Vec::new(),
		}],
	}
}

#[test]
fn hooks_do_nothing_without_an_agent() {
	registry::reset();
	let owner = Owner::new();
	owner.set();
	let count = RwSignal::new(1_usize);
	expose("count", count);
	track_router(&Router::<Page>::new(Page::Home));
	#[cfg(feature = "form")]
	crate::track_form("signup", &leptoskit_form::FormState::default());
	#[cfg(feature = "query")]
	crate::track_queries(&leptoskit_query::QueryClient::new());

	assert_eq!(registry::len(), 0);
	count.set(2);
	let changes = registry::collect();
	assert_eq!(changes.events, []);
	assert!(!changes.removed);
	let error = registry::apply(&set_state("count", json!(3))).unwrap_err();
	assert!(error.contains("devtools::expose"), "{error}");
	assert_eq!(count.get_untracked(), 2);

	// An agent that arrives later finds nothing the earlier calls left.
	let agent = Rc::new(FakeAgent::default());
	registry::install(agent.clone());
	assert_eq!(agent.wakes.get(), 0);
	assert_eq!(registry::collect().events, []);
}

#[test]
fn sessions_declare_what_the_hooks_can_feed() {
	let capabilities = registry::capabilities();
	for capability in [
		Capability::Navigation,
		Capability::State,
		Capability::Editing,
	] {
		assert!(capabilities.contains(&capability), "{capability:?}");
	}
	assert_eq!(
		capabilities.contains(&Capability::Forms),
		cfg!(feature = "form")
	);
	for capability in [Capability::Queries, Capability::Mutations] {
		assert_eq!(
			capabilities.contains(&capability),
			cfg!(feature = "query"),
			"{capability:?}"
		);
	}
}

#[test]
fn the_route_is_reported_and_navigated() {
	let (agent, _owner) = agent();
	let router = Router::<Page>::new(Page::Home);
	track_router(&router);
	assert!(agent.wakes.get() > 0, "tracking wakes the agent");
	assert_eq!(registry::collect().events, [route("/")]);
	assert_eq!(registry::collect().events, [], "nothing changed");

	router.navigate(Page::Settings);
	assert_eq!(registry::collect().events, [route("/settings")]);

	registry::apply(&Command::Navigate { href: "/".into() }).unwrap();
	assert_eq!(untrack(|| router.current()), Page::Home);
	assert_eq!(registry::collect().events, [route("/")]);

	let error = registry::apply(&Command::Navigate {
		href: "/nowhere".into(),
	})
	.unwrap_err();
	assert!(error.contains("/nowhere"), "{error}");
	assert_eq!(untrack(|| router.current()), Page::Home);
}

#[test]
fn navigating_without_a_tracked_router_says_how_to_track_one() {
	let (_agent, _owner) = agent();
	let error = registry::apply(&Command::Navigate {
		href: "/settings".into(),
	})
	.unwrap_err();
	assert!(error.contains("devtools::track_router"), "{error}");
}

#[test]
fn exposed_state_is_reported_and_set() {
	let (agent, _owner) = agent();
	let count = RwSignal::new(2_usize);
	expose("cart.count", count);
	assert_eq!(
		registry::collect().events,
		[state("cart.count", "usize", json!(2))]
	);

	// A write wakes the agent once, however many follow before it flushes.
	let wakes = agent.wakes.get();
	count.set(3);
	count.set(4);
	assert_eq!(agent.wakes.get(), wakes + 1);
	assert_eq!(
		registry::collect().events,
		[state("cart.count", "usize", json!(4))]
	);

	registry::apply(&set_state("cart.count", json!(5))).unwrap();
	assert_eq!(count.get_untracked(), 5);
	assert_eq!(
		registry::collect().events,
		[state("cart.count", "usize", json!(5))]
	);

	let error = registry::apply(&set_state("cart.count", json!("five"))).unwrap_err();
	assert!(error.contains("usize"), "the error names the type: {error}");
	let mut truncated = Value::from_json("usize", json!(6));
	truncated.truncated = true;
	let error = registry::apply(&Command::SetState {
		name: "cart.count".into(),
		value: truncated,
	})
	.unwrap_err();
	assert!(error.contains("truncated"), "{error}");
	assert_eq!(count.get_untracked(), 5);

	let error = registry::apply(&set_state("missing", json!(1))).unwrap_err();
	assert!(error.contains("\"missing\""), "{error}");
}

#[test]
fn exposing_a_name_again_replaces_the_signal() {
	let (_agent, _owner) = agent();
	let first = ArcRwSignal::new(Draft {
		title: "Field notes".into(),
		tags: Vec::new(),
	});
	let second = ArcRwSignal::new(Draft {
		title: "Reading list".into(),
		tags: vec!["books".into()],
	});
	expose("draft", first.clone());
	expose("draft", second.clone());
	assert_eq!(registry::len(), 1);
	assert_eq!(
		registry::collect().events,
		[state(
			"draft",
			"Draft",
			json!({"title": "Reading list", "tags": ["books"]})
		)]
	);

	// The first signal no longer reports.
	first.set(Draft {
		title: "Gone".into(),
		tags: Vec::new(),
	});
	assert_eq!(registry::collect().events, []);

	registry::apply(&Command::SetState {
		name: "draft".into(),
		value: Value::from_json("Draft", json!({"title": "Edited", "tags": []})),
	})
	.unwrap();
	assert_eq!(second.get_untracked().title, "Edited");
}

#[test]
fn state_leaves_the_panel_with_its_component() {
	let (_agent, owner) = agent();
	let component = owner.child();
	component.with(|| expose("count", RwSignal::new(1_usize)));
	let changes = registry::collect();
	assert_eq!(changes.events.len(), 1);
	assert!(!changes.removed);

	component.cleanup();
	assert_eq!(registry::len(), 0);
	let changes = registry::collect();
	assert!(changes.removed, "only a snapshot can remove a value");
	assert_eq!(registry::snapshot(tree("View")).state, []);
}

#[test]
fn a_snapshot_carries_every_value() {
	let (_agent, _owner) = agent();
	let router = Router::<Page>::new(Page::Settings);
	track_router(&router);
	expose("count", RwSignal::new(7_usize));
	registry::collect();
	let snapshot = registry::snapshot(tree("View"));
	assert_eq!(snapshot.tree, tree("View"));
	assert_eq!(snapshot.route.as_deref(), Some("/settings"));
	assert_eq!(snapshot.state.len(), 1);
	assert_eq!(snapshot.state[0].name, "count");
	assert_eq!(snapshot.state[0].value, Value::from_json("usize", json!(7)));
}

#[test]
fn the_outbox_sends_a_snapshot_then_only_what_changed() {
	let (_agent, _owner) = agent();
	let count = RwSignal::new(1_usize);
	expose("count", count);
	let mut outbox = Outbox::default();
	let untouched = || -> Tree { panic!("the tree is read only when it may have changed") };

	// A new session starts with a snapshot of the tree and every value.
	let batch = outbox.batch(false, || tree("View")).unwrap();
	let [Event::Snapshot { snapshot }] = batch.events.as_slice() else {
		panic!("a snapshot first: {:?}", batch.events);
	};
	assert_eq!(snapshot.state[0].value, Value::from_json("usize", json!(1)));
	outbox.sent(batch);
	assert!(outbox.batch(false, untouched).is_none(), "nothing changed");

	// A value changes: only that value, and the tree isn't read.
	count.set(2);
	let batch = outbox.batch(false, untouched).unwrap();
	assert_eq!(batch.events, [state("count", "usize", json!(2))]);
	outbox.sent(batch);

	// The DOM changed but the tree didn't: still only the value.
	count.set(3);
	let batch = outbox.batch(true, || tree("View")).unwrap();
	assert_eq!(batch.events, [state("count", "usize", json!(3))]);
	outbox.sent(batch);

	// The tree changed: a snapshot, which carries the latest values.
	count.set(4);
	let batch = outbox.batch(true, || tree("Text")).unwrap();
	let [Event::Snapshot { snapshot }] = batch.events.as_slice() else {
		panic!("the tree changed: {:?}", batch.events);
	};
	assert_eq!(snapshot.tree, tree("Text"));
	assert_eq!(snapshot.state[0].value, Value::from_json("usize", json!(4)));
	outbox.sent(batch);

	// A failed post, a new session, or `RequestSnapshot` resets the outbox:
	// the next batch is a snapshot even though nothing changed.
	outbox.reset();
	let batch = outbox.batch(false, || tree("Text")).unwrap();
	assert!(matches!(batch.events.as_slice(), [Event::Snapshot { .. }]));
}

#[test]
fn a_value_that_goes_away_sends_a_snapshot() {
	let (_agent, owner) = agent();
	let component = owner.child();
	component.with(|| expose("count", RwSignal::new(1_usize)));
	let mut outbox = Outbox::default();
	let batch = outbox.batch(false, || tree("View")).unwrap();
	outbox.sent(batch);

	component.cleanup();
	let batch = outbox.batch(false, || tree("View")).unwrap();
	let [Event::Snapshot { snapshot }] = batch.events.as_slice() else {
		panic!("a removal needs a snapshot: {:?}", batch.events);
	};
	assert_eq!(snapshot.state, []);
}

#[cfg(feature = "form")]
mod forms {
	use leptoskit_form::FieldSpec;
	use leptoskit_form::Form;
	use leptoskit_form::FormState;

	use super::*;
	use crate::track_form;

	fn form(name: &str, json: serde_json::Value) -> Event {
		Event::FormChanged {
			name: name.into(),
			value: Value::from_json("FormState", json),
		}
	}

	#[test]
	fn forms_report_values_errors_and_touched_fields() {
		let (agent, _owner) = agent();
		let signup = Form::new("signup").with_field(
			FieldSpec::new("email", "Email address")
				.required()
				.validate(|value| (!value.contains('@')).then(|| "That address has no @.".into())),
		);
		let state = FormState::initial(&signup);
		track_form("signup", &state);
		assert_eq!(
			registry::collect().events,
			[form(
				"signup",
				json!({"values": {"email": ""}, "errors": {}, "touched": [], "submitted": false})
			)]
		);

		// The same state again only costs a comparison.
		let wakes = agent.wakes.get();
		track_form("signup", &state);
		assert_eq!(agent.wakes.get(), wakes);
		assert_eq!(registry::collect().events, []);

		let state = state
			.for_change(&signup, "email", "ifiok")
			.for_blur(&signup, "email");
		track_form("signup", &state);
		assert_eq!(registry::len(), 1, "one source per form");
		assert_eq!(
			registry::collect().events,
			[form(
				"signup",
				json!({
					"values": {"email": "ifiok"},
					"errors": {"email": "That address has no @."},
					"touched": ["email"],
					"submitted": false,
				})
			)]
		);
	}

	#[test]
	fn forms_are_read_only() {
		let (_agent, _owner) = agent();
		track_form("signup", &FormState::default());
		registry::collect();
		let error = registry::apply(&set_state("signup", json!({}))).unwrap_err();
		assert!(error.contains("devtools::expose"), "{error}");
	}
}

#[cfg(feature = "query")]
mod queries {
	use std::time::Duration;

	use leptoskit_query::MutationScope;
	use leptoskit_query::NetworkMode;
	use leptoskit_query::QueryClient;
	use leptoskit_query::QueryDefaults;
	use leptoskit_query::QueryScope;
	use leptoskit_query::RefetchOn;
	use leptoskit_query::use_mutation_in;

	use super::*;
	use crate::track_queries;

	/// A client whose queries ignore focus and the network, tracked by the
	/// fake agent.
	fn client() -> QueryClient {
		let _ = any_spawner::Executor::init_tokio();
		let client = QueryClient::new().with_defaults(
			QueryDefaults::new()
				.with_refetch_on_window_focus(RefetchOn::Never)
				.with_refetch_on_reconnect(RefetchOn::Never)
				.with_network_mode(NetworkMode::Always),
		);
		track_queries(&client);
		client
	}

	fn clips() -> QueryScope<(), Vec<String>, String> {
		QueryScope::new(|()| async { Ok(vec!["Fetched".to_owned()]) })
			.with_name("clips")
			.with_stale_time(Duration::from_secs(60))
			.with_ssr()
	}

	/// A scope without a codec.
	fn doubled() -> QueryScope<u32, u32, String> {
		QueryScope::new(|id| async move { Ok(id * 2) }).with_name("doubled")
	}

	/// The one query event of a collect.
	fn changed() -> (String, Value) {
		let events = registry::collect().events;
		match events.as_slice() {
			[Event::QueryChanged { key, value }] => (key.clone(), value.clone()),
			_ => panic!("one query changed: {events:?}"),
		}
	}

	async fn wait_for(label: &str, condition: impl Fn() -> bool) {
		for _ in 0..200 {
			if condition() {
				return;
			}
			tokio::time::sleep(Duration::from_millis(5)).await;
		}
		panic!("timed out waiting for: {label}");
	}

	#[tokio::test]
	async fn queries_report_their_state_and_data() {
		let (_agent, _owner) = agent();
		let client = client();
		assert_eq!(registry::collect().events, [], "an empty cache");

		client.set_query_data(&clips(), (), vec!["Field notes".to_owned()]);
		let (key, value) = changed();
		assert_eq!(key, "clips");
		assert_eq!(value.type_name, "QueryState<Vec<String>>");
		assert!(!value.truncated);
		assert_eq!(value.json["scope"], "clips");
		assert_eq!(value.json["key"], "()");
		assert_eq!(value.json["status"], "success");
		assert_eq!(value.json["fetch_status"], "idle");
		assert_eq!(value.json["stale"], false);
		assert_eq!(value.json["invalidated"], false);
		assert_eq!(value.json["observers"], 0);
		assert!(value.json["data_updated_at"].is_u64());
		assert!(value.json["error_updated_at"].is_null());
		assert_eq!(value.json["data"], json!(["Field notes"]));
		assert!(value.json.get("read_only").is_none());

		let snapshot = registry::snapshot(tree("View"));
		assert_eq!(snapshot.queries.len(), 1);
		assert_eq!(snapshot.queries[0].value, value);
	}

	#[tokio::test]
	async fn queries_without_a_codec_are_read_only() {
		let (_agent, _owner) = agent();
		let client = client();
		client.set_query_data(&doubled(), 4, 8);
		let (key, value) = changed();
		assert_eq!(key, "doubled 4");
		assert_eq!(value.type_name, "QueryState<u32>");
		assert_eq!(value.json["status"], "success");
		assert!(value.json["data"].is_null());
		assert!(
			value.json["read_only"]
				.as_str()
				.is_some_and(|reason| reason.contains("no codec")),
			"{value:?}"
		);

		let error = registry::apply(&Command::SetQueryData {
			key: "doubled 4".into(),
			value: Value::from_json("QueryState<u32>", json!({"data": 9})),
		})
		.unwrap_err();
		assert!(error.contains("codec"), "{error}");
		assert_eq!(client.get_query_data(&doubled(), &4), Some(8));

		// Refetching and invalidating need no codec.
		registry::apply(&Command::InvalidateQuery {
			key: "doubled 4".into(),
		})
		.unwrap();
		let (_, value) = changed();
		assert_eq!(value.json["invalidated"], true);
	}

	#[tokio::test]
	async fn edits_go_through_the_codec() {
		let (_agent, _owner) = agent();
		let client = client();
		client.set_query_data(&clips(), (), vec!["Field notes".to_owned()]);
		let (_, value) = changed();

		// The DevTools send back the whole value they showed, edited.
		let mut edited = value.clone();
		edited.json["data"] = json!(["Edited"]);
		registry::apply(&Command::SetQueryData {
			key: "clips".into(),
			value: edited,
		})
		.unwrap();
		assert_eq!(
			client.get_query_data(&clips(), &()),
			Some(vec!["Edited".to_owned()])
		);
		let (_, value) = changed();
		assert_eq!(value.json["data"], json!(["Edited"]));

		for (json, reason) in [
			(json!({"data": 7}), "codec refused"),
			(json!(["Edited"]), "no `data`"),
		] {
			let error = registry::apply(&Command::SetQueryData {
				key: "clips".into(),
				value: Value::from_json("QueryState<Vec<String>>", json),
			})
			.unwrap_err();
			assert!(error.contains(reason), "{error}");
		}

		let mut truncated = value;
		truncated.truncated = true;
		let error = registry::apply(&Command::SetQueryData {
			key: "clips".into(),
			value: truncated,
		})
		.unwrap_err();
		assert!(error.contains("truncated"), "{error}");
		assert_eq!(
			client.get_query_data(&clips(), &()),
			Some(vec!["Edited".to_owned()])
		);
	}

	#[tokio::test]
	async fn queries_are_invalidated_and_refetched() {
		let (_agent, _owner) = agent();
		let client = client();
		client.set_query_data(&clips(), (), vec!["Field notes".to_owned()]);
		changed();

		registry::apply(&Command::InvalidateQuery {
			key: "clips".into(),
		})
		.unwrap();
		let (_, value) = changed();
		assert_eq!(value.json["invalidated"], true);
		assert_eq!(value.json["stale"], true);

		registry::apply(&Command::RefetchQuery {
			key: "clips".into(),
		})
		.unwrap();
		let (_, value) = changed();
		assert_eq!(value.json["fetch_status"], "fetching");
		wait_for("the refetch", || {
			client.get_query_data(&clips(), &()) == Some(vec!["Fetched".to_owned()])
		})
		.await;
		let (_, value) = changed();
		assert_eq!(value.json["fetch_status"], "idle");
		assert_eq!(value.json["invalidated"], false);
		assert_eq!(value.json["data"], json!(["Fetched"]));

		let error = registry::apply(&Command::RefetchQuery {
			key: "missing".into(),
		})
		.unwrap_err();
		assert!(error.contains("devtools::track_queries"), "{error}");
	}

	#[tokio::test]
	async fn large_data_is_shown_by_its_size() {
		let (_agent, _owner) = agent();
		let client = client();
		let big = "x".repeat(leptoskit_devtools_protocol::MAX_VALUE_BYTES);
		client.set_query_data(&clips(), (), vec![big.clone(), big]);
		let (_, value) = changed();
		assert!(value.truncated);
		assert_eq!(value.json["status"], "success", "the status stays readable");
		assert_eq!(value.json["data"]["truncated"], true);
		assert!(value.json["data"]["bytes"].as_u64().unwrap() > 2 * 64 * 1024);
	}

	#[tokio::test]
	async fn a_removed_query_leaves_with_a_snapshot() {
		let (_agent, _owner) = agent();
		let client = client();
		client.set_query_data(&clips(), (), vec!["Field notes".to_owned()]);
		changed();

		client.clear();
		let changes = registry::collect();
		assert!(changes.removed);
		assert_eq!(registry::snapshot(tree("View")).queries, []);
	}

	#[tokio::test]
	async fn mutations_report_their_status() {
		let (_agent, _owner) = agent();
		let client = client();
		let scope: MutationScope<String, usize, String> =
			MutationScope::new(|title: String| async move { Ok(title.len()) })
				.with_name("create_project");
		let create = use_mutation_in(client.clone(), scope);
		assert_eq!(create.mutate_async("Field notes".to_owned()).await, Ok(11));

		let events = registry::collect().events;
		let [Event::MutationChanged { key, value }] = events.as_slice() else {
			panic!("one mutation changed: {events:?}");
		};
		assert!(key.starts_with("create_project #"), "{key}");
		assert_eq!(value.type_name, "MutationState<String, usize>");
		assert_eq!(value.json["status"], "success");
		assert_eq!(value.json["scope"], "create_project");
		assert_eq!(value.json["paused"], false);
		assert!(value.json["submitted_at"].is_u64());
	}

	#[tokio::test]
	async fn queries_that_print_the_same_get_numbered() {
		let (_agent, _owner) = agent();
		let client = client();
		// `same` with the key `1`, and `same 1` with no key, both read
		// `same 1`.
		let first: QueryScope<u32, u32, String> =
			QueryScope::new(|id| async move { Ok(id) }).with_name("same");
		let second: QueryScope<(), u32, String> =
			QueryScope::new(|()| async move { Ok(0) }).with_name("same 1");
		client.set_query_data(&first, 1, 1);
		client.set_query_data(&second, (), 2);
		let mut keys: Vec<String> = registry::collect()
			.events
			.iter()
			.filter_map(|event| {
				match event {
					Event::QueryChanged { key, .. } => Some(key.clone()),
					_ => None,
				}
			})
			.collect();
		keys.sort();
		assert_eq!(keys, ["same 1", "same 1 #2"]);

		// Each number reaches its own query.
		registry::apply(&Command::InvalidateQuery {
			key: "same 1 #2".into(),
		})
		.unwrap();
		let (key, value) = changed();
		assert_eq!(key, "same 1 #2");
		assert_eq!(value.json["invalidated"], true);
	}
}
