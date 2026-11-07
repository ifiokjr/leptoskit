//! The renderer's own surface: configuring it, resetting it, and what the
//! reference teaches.

use std::collections::BTreeMap;

use leptoskit_agent_ui::AgentUi;
use leptoskit_agent_ui::Component;
use leptoskit_agent_ui::Limits;
use leptoskit_agent_ui::Prop;
use leptoskit_agent_ui::Value;
use leptoskit_native::components::Text;
use leptoskit_native::reactive::Get;
use leptoskit_native::reactive::RwSignal;
use leptoskit_testing::Queries;
use leptoskit_testing::WidgetTester;

#[test]
fn a_renderer_describes_and_defaults_itself() {
	let ui = AgentUi::default();
	let debug = format!("{ui:?}");
	assert!(debug.contains("AgentUi"), "{debug}");
	assert!(debug.contains("View"), "{debug}");
	assert!(debug.contains("Limits"), "{debug}");
}

#[test]
fn configuring_a_shared_renderer_keeps_the_other_handles() {
	let ui = AgentUi::new();
	let clone = ui.clone();
	let configured = ui.clone().limits(Limits {
		max_nodes: 10,
		..Limits::default()
	});
	// The original and its clone keep the default limits.
	assert_eq!(
		clone.check(&"Line.\n\n".repeat(20)),
		[] as [leptoskit_agent_ui::Diagnostic; 0]
	);
	assert!(
		configured
			.check(&"Line.\n\n".repeat(20))
			.iter()
			.any(|diagnostic| diagnostic.message.contains("limited to 10"))
	);
}

#[test]
fn a_component_can_be_removed() {
	let ui = AgentUi::new().without("Stat");
	assert!(
		ui.check("<Stat label=\"x\" value={1} />\n")[0]
			.message
			.contains("there is no component <Stat>")
	);
}

#[test]
fn component_names_must_start_uppercase() {
	let result = std::panic::catch_unwind(|| {
		AgentUi::new().component(Component::new("lowercase", "Nope.", |_props, _cx| {
			Ok(Text("no"))
		}))
	});
	assert!(result.is_err());
}

#[test]
fn resetting_forgets_document_variables_and_keeps_data() {
	let ui = AgentUi::new().data("income", 4200);
	ui.render("<Var name=\"rent\" value={1800} />\n<Let name=\"yearly\" value={rent * 12} />\n");
	assert_eq!(ui.value("rent"), Some(Value::Number(1800.0)));
	assert_eq!(ui.value("yearly"), Some(Value::Number(21600.0)));
	ui.reset();
	assert_eq!(ui.value("rent"), None);
	assert_eq!(ui.value("yearly"), None);
	assert_eq!(ui.value("income"), Some(Value::Number(4200.0)));
}

#[test]
fn set_data_updates_a_variable_already_read() {
	let ui = AgentUi::new().data("income", 4200);
	ui.render("<Let name=\"half\" value={income / 2} />\n");
	assert_eq!(ui.value("half"), Some(Value::Number(2100.0)));
	ui.set_data("income", 1000);
	assert_eq!(ui.value("half"), Some(Value::Number(500.0)));
}

#[test]
fn a_view_shows_the_text_it_is_given() {
	let owner = reactive_graph::owner::Owner::new();
	owner.with(|| {
		let answer = RwSignal::new(String::from("# Ready\n\nHere it is."));
		let ui = AgentUi::new();
		let view = ui.view(move || answer.get());
		let tester = WidgetTester::new(move || view.clone(), 390.0, 400.0).unwrap();
		tester.get_by_text("Ready").unwrap();
		tester.get_by_text("Here it is.").unwrap();
	});
}

#[test]
fn the_reference_describes_shared_data() {
	let records = vec![Value::Record(BTreeMap::from([(
		"payee".to_owned(),
		Value::from("Grocer"),
	)]))];

	let ui = AgentUi::new()
		.data("income", 4200)
		.data("transactions", records)
		.data("empty", Vec::<Value>::new())
		.data("one", vec![Value::Number(1.0)]);
	let reference = ui.reference();
	assert!(reference.contains("`income`: a number"), "{reference}");
	assert!(
		reference.contains("`transactions`: a list of 1 record with fields `payee`"),
		"{reference}"
	);
	assert!(reference.contains("`empty`: an empty list"), "{reference}");
	assert!(
		reference.contains("`one`: a list of 1 number"),
		"{reference}"
	);

	let two = AgentUi::new().data(
		"rows",
		vec![
			Value::Record(BTreeMap::from([("a".to_owned(), Value::from(1.0))])),
			Value::Record(BTreeMap::from([("a".to_owned(), Value::from(2.0))])),
		],
	);
	assert!(
		two.reference()
			.contains("a list of 2 records with fields `a`"),
		"{}",
		two.reference()
	);

	let record = AgentUi::new().data("budget", BTreeMap::from([("rent".to_owned(), 1_i32)]));
	assert!(
		record
			.reference()
			.contains("`budget`: a record with fields `rent`"),
		"{}",
		record.reference()
	);

	let no_data = AgentUi::new().reference();
	assert!(
		no_data.contains("The application shares no data right now."),
		"{no_data}"
	);
}

#[test]
fn the_reference_reports_whether_images_show() {
	let blocked = AgentUi::new().reference();
	assert!(blocked.contains("Images are not shown"), "{blocked}");

	let allowed = AgentUi::new().allow_images(|_| true).reference();
	assert!(
		allowed.contains("Images show only from addresses this application allows."),
		"{allowed}"
	);
}

#[test]
fn the_reference_lists_components_without_props() {
	let reference = AgentUi::new().reference();
	assert!(reference.contains("### `<Divider>`"), "{reference}");
	assert!(reference.contains("- No props."), "{reference}");
	assert!(reference.contains("Content: The text"), "{reference}");
}

#[test]
fn props_are_described_with_their_kinds_and_defaults() {
	let ui = AgentUi::new().component(
		Component::new("Everything", "One of every prop kind.", |_props, _cx| {
			Ok(Text("everything"))
		})
		.prop(Prop::number("a-number", "A number."))
		.prop(Prop::text("some-text", "Some text.").required())
		.prop(Prop::bool("a-flag", "A flag.").default("false"))
		.prop(Prop::list("a-list", "A list."))
		.prop(Prop::any("anything", "Anything."))
		.prop(Prop::variable("a-variable", "A variable.")),
	);
	let reference = ui.reference();
	assert!(
		reference.contains("`a-number` (number): A number."),
		"{reference}"
	);
	assert!(
		reference.contains("`some-text` (text), required: Some text."),
		"{reference}"
	);
	assert!(
		reference.contains("`a-flag` (boolean), default `false`: A flag."),
		"{reference}"
	);
	assert!(
		reference.contains("`a-list` (list): A list."),
		"{reference}"
	);
	assert!(
		reference.contains("`anything` (any): Anything."),
		"{reference}"
	);
	assert!(
		reference.contains("`a-variable` (variable name): A variable."),
		"{reference}"
	);
}

#[test]
fn the_reference_never_describes_a_component_without_children_props_or_example() {
	let reference = AgentUi::new().reference();
	assert!(reference.contains("## Example"), "{reference}");
	assert!(reference.contains("## Rules"), "{reference}");
}

#[test]
fn a_component_names_and_describes_itself() {
	let component = Component::new("Named", "A named component.", |_props, _cx| {
		Ok(Text("named"))
	})
	.prop(Prop::text("one", "First."));
	assert_eq!(component.name(), "Named");
	let debug = format!("{component:?}");
	assert!(debug.contains("Named"), "{debug}");
	assert!(debug.contains("one"), "{debug}");
}

#[test]
#[allow(
	clippy::cognitive_complexity,
	reason = "one test walks every conversion and accessor"
)]
fn values_convert_from_the_types_applications_hold() {
	assert_eq!(Value::from(1_i64), Value::Number(1.0));
	assert_eq!(Value::from(1_i32), Value::Number(1.0));
	assert_eq!(Value::from(true), Value::Bool(true));
	assert_eq!(Value::from(String::from("s")), Value::from("s"));
	assert_eq!(
		Value::from(BTreeMap::from([("a".to_owned(), 1_i32)])),
		Value::Record(BTreeMap::from([("a".to_owned(), Value::Number(1.0))]))
	);

	let value = Value::from(vec![1_i32, 2]);
	assert_eq!(value, Value::from(vec![1.0, 2.0]));

	let record = Value::Record(BTreeMap::from([
		("a".to_owned(), Value::Number(1.0)),
		("b".to_owned(), Value::from("two")),
	]));
	assert_eq!(record.to_string(), "a: 1, b: two");
	assert_eq!(format!("{}", Value::Null), "");
	assert_eq!(
		Value::List(vec![Value::Number(1.0), Value::from("x")]).to_string(),
		"1, x"
	);
	assert_eq!(Value::Bool(true).as_number(), Some(1.0));
	assert_eq!(Value::Bool(false).as_number(), Some(0.0));
	assert_eq!(Value::Null.as_number(), None);
	assert_eq!(Value::from("$1,800").as_number(), Some(1800.0));
	assert_eq!(Value::from(" ").as_number(), None);
	assert_eq!(Value::from("inf").as_number(), None);
	assert!(!Value::from(0.0).truthy());
	assert!(!Value::from(f64::NAN).truthy());
	assert!(Value::from(1.0).truthy());
	assert!(!Value::Null.truthy());
	assert!(!Value::from("").truthy());
	assert!(!Value::List(Vec::new()).truthy());
	assert!(!Value::Record(BTreeMap::new()).truthy());
	assert_eq!(Value::Null.kind(), "null");
	assert_eq!(Value::Bool(true).kind(), "boolean");
	assert_eq!(Value::Number(1.0).kind(), "number");
	assert_eq!(Value::from("s").kind(), "text");
	assert_eq!(Value::List(Vec::new()).kind(), "list");
	assert_eq!(Value::Record(BTreeMap::new()).kind(), "record");
}

#[test]
fn values_hash_by_their_contents() {
	fn hash(value: &Value) -> u64 {
		use std::hash::Hash;
		use std::hash::Hasher;

		let mut hasher = std::collections::hash_map::DefaultHasher::new();
		value.hash(&mut hasher);
		hasher.finish()
	}

	assert_ne!(hash(&Value::Null), hash(&Value::from(1.0)));
	assert_ne!(hash(&Value::from(1.0)), hash(&Value::from(2.0)));
	assert_ne!(hash(&Value::from("a")), hash(&Value::from("b")));
	assert_ne!(
		hash(&Value::List(vec![Value::from(1.0)])),
		hash(&Value::List(vec![Value::from(2.0)]))
	);
	assert_ne!(
		hash(&Value::Record(BTreeMap::from([(
			"a".to_owned(),
			Value::from(1.0)
		)]))),
		hash(&Value::Record(BTreeMap::from([(
			"a".to_owned(),
			Value::from(2.0)
		)])))
	);
}

#[test]
fn numbers_are_displayed_the_way_documents_show_them() {
	assert_eq!(Value::Number(1800.0).to_string(), "1800");
	assert_eq!(Value::Number(0.25).to_string(), "0.25");
	assert_eq!(Value::Number(f64::NAN).to_string(), "—");
}
