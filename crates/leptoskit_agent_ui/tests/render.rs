//! Documents rendered on a headless surface, read and pressed the way a user
//! would.

use std::sync::Arc;
use std::sync::Mutex;

use leptoskit_agent_ui::Action;
use leptoskit_agent_ui::AgentUi;
use leptoskit_agent_ui::Component;
use leptoskit_agent_ui::Limits;
use leptoskit_agent_ui::Prop;
use leptoskit_agent_ui::Value;
use leptoskit_agent_ui::Widgets;
use leptoskit_native::Color;
use leptoskit_native::NativeKind;
use leptoskit_native::components::Text;
use leptoskit_testing::Finder;
use leptoskit_testing::Queries;
use leptoskit_testing::WidgetTester;
use leptoskit_yoga::NodeId;

fn mount(ui: &AgentUi, source: &'static str) -> WidgetTester {
	let ui = ui.clone();
	WidgetTester::new(move || ui.render(source).element, 390.0, 2000.0).unwrap()
}

fn button(label: &str) -> Finder {
	Finder::role(NativeKind::Button, label)
}

#[test]
fn renders_markdown() {
	let ui = AgentUi::new();
	let rendered = ui.render(
		"# May\n\nYou spent **less** than in April.\n\n- Rent\n- Groceries\n\n| Category | Spent \
		 |\n| --- | ---: |\n| Rent | 1800 |\n",
	);
	assert!(
		rendered.diagnostics.is_empty(),
		"{:?}",
		rendered.diagnostics
	);

	let tester = mount(
		&ui,
		"# May\n\nYou spent **less** than in April.\n\n- Rent\n- Groceries\n\n| Category | Spent \
		 |\n| --- | ---: |\n| Rent | 1800 |\n",
	);
	tester.get_by_text("May").unwrap();
	tester.get_by_text("You spent less than in April.").unwrap();
	tester.get_by_text("Groceries").unwrap();
	tester.get_by_text("Category").unwrap();
	tester.get_by_text("1800").unwrap();
}

#[test]
fn formulas_read_variables_and_application_data() {
	let ui = AgentUi::new().data("income", 4200);
	let tester = mount(
		&ui,
		"<Var name=\"rent\" value={1800} />\n<Let name=\"left\" value={income - rent} />\n\nLeft \
		 over: {money(left)} of {money(income)}.\n",
	);
	tester.get_by_text("Left over: $2,400 of $4,200.").unwrap();
	assert_eq!(ui.value("left"), Some(Value::Number(2400.0)));
}

#[test]
fn a_stepper_changes_what_reads_its_variable() {
	let ui = AgentUi::new().data("income", 4200);
	let mut tester = mount(
		&ui,
		"<Var name=\"rent\" value={1800} />\n\n<Stepper bind=\"rent\" step={100} max={1900} \
		 label=\"Rent\" format=\"money\" />\n\nLeft: {money(income - rent)}\n",
	);
	tester.get_by_text("$1,800").unwrap();
	tester.get_by_text("Left: $2,400").unwrap();

	tester.tap(&Finder::label("Increase by $100")).unwrap();
	tester.get_by_text("$1,900").unwrap();
	tester.get_by_text("Left: $2,300").unwrap();

	// At the maximum, the button that would pass it is disabled.
	let increase = tester.get_by_label("Increase by $100").unwrap();
	assert_eq!(increase.props.enabled, Some(false));

	tester.tap(&Finder::label("Decrease by $100")).unwrap();
	tester.get_by_text("Left: $2,400").unwrap();
	assert_eq!(ui.inputs().get("rent"), Some(&Value::Number(1800.0)));
}

#[test]
fn buttons_send_actions_with_the_users_state() {
	let actions = Arc::new(Mutex::new(Vec::<Action>::new()));
	let ui = AgentUi::new().on_action({
		let actions = Arc::clone(&actions);
		move |action| actions.lock().unwrap().push(action.clone())
	});
	let mut tester = mount(
		&ui,
		"<Var name=\"plan\" value=\"steady\" />\n<Var name=\"rent\" value={1800} />\n\n<Choice \
		 bind=\"plan\" options={[\"steady\", \"fast\"]} labels={[\"Steady\", \"Fast\"]} \
		 />\n\n<Button action=\"apply\" value={rent * 12} label={\"Apply \" + plan} />\n",
	);

	tester.tap(&button("Fast")).unwrap();
	assert_eq!(ui.inputs().get("plan"), Some(&Value::from("fast")));

	tester.tap(&button("Apply fast")).unwrap();
	let actions = actions.lock().unwrap();
	let action = actions.last().expect("the button sent an action");
	assert_eq!(action.name, "apply");
	assert_eq!(action.value, Value::Number(21600.0));
	assert_eq!(action.inputs.get("plan"), Some(&Value::from("fast")));
}

#[test]
fn buttons_without_a_handler_are_disabled() {
	let ui = AgentUi::new();
	let tester = mount(&ui, "<Button action=\"apply\">Apply</Button>\n");
	let widget = tester.get_by_text("Apply").unwrap();
	assert_eq!(widget.props.enabled, Some(false));
}

#[test]
fn a_button_can_set_a_variable_without_the_application() {
	let ui = AgentUi::new();
	let mut tester = mount(
		&ui,
		"<Var name=\"view\" value=\"month\" />\n\n<Button set=\"view\" to=\"year\">By \
		 year</Button>\n\n<Show when={view == \"year\"}>\n\nShowing the year.\n\n</Show>\n",
	);
	assert!(tester.query_by_text("Showing the year.").unwrap().is_none());
	tester.tap(&button("By year")).unwrap();
	tester.get_by_text("Showing the year.").unwrap();
}

#[test]
fn text_fields_write_variables() {
	let ui = AgentUi::new();
	let mut tester = mount(
		&ui,
		"<Var name=\"saving\" value={0} />\n\n<NumberField bind=\"saving\" label=\"Monthly \
		 saving\" />\n\nA year: {money(saving * 12)}\n",
	);
	tester
		.enter_text(&Finder::label("Monthly saving"), "$250")
		.unwrap();
	tester.get_by_text("A year: $3,000").unwrap();
}

#[test]
fn charts_render_every_kind() {
	let ui = AgentUi::new();
	for kind in ["bar", "line", "area", "pie", "donut"] {
		let source = format!(
			"<Chart type=\"{kind}\" title=\"Spending\" labels={{[\"Rent\", \"Food\", \"Fun\"]}} \
			 values={{[1800, 600, 200]}} format=\"money\" />\n"
		);
		let rendered = ui.render(&source);
		assert!(
			rendered.diagnostics.is_empty(),
			"{kind}: {:?}",
			rendered.diagnostics
		);
	}

	let tester = mount(
		&ui,
		"<Chart type=\"bar\" title=\"Cash flow\" labels={[\"Jan\", \"Feb\"]}>\n  <Series \
		 name=\"In\" values={[4200, 4300]} />\n  <Series name=\"Out\" values={[-3900, -4500]} \
		 />\n</Chart>\n",
	);
	tester.get_by_text("Cash flow").unwrap();
	tester.get_by_text("In").unwrap();
	tester.get_by_text("Out").unwrap();
	tester.get_by_text("Jan").unwrap();
}

#[test]
fn charts_explain_bad_data() {
	let ui = AgentUi::new();
	let cases = [
		(
			"<Chart values={[1, 2]} labels={[\"a\"]} />",
			"1 labels for 2 values",
		),
		("<Chart type=\"pie\" values={[1, -2]} />", "negative"),
		("<Chart type=\"radar\" values={[1]} />", "`type`"),
		("<Chart />", "needs `values`"),
		("<Chart values={range(600)} />", "at most 500 points"),
	];
	for (source, expected) in cases {
		let diagnostics = ui.check(source);
		assert!(
			diagnostics
				.iter()
				.any(|diagnostic| diagnostic.message.contains(expected)),
			"{source}: {diagnostics:?}"
		);
	}
}

#[test]
fn diagnostics_point_at_the_problem() {
	let ui = AgentUi::new();
	let diagnostics = ui.check("Fine.\n\n<Stat label=\"Spent\" value={missing * 2} />\n");
	assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
	assert_eq!(diagnostics[0].line, 3);
	assert!(diagnostics[0].message.contains("no variable `missing`"));

	let diagnostics = ui.check("<Sparkle />\n");
	assert!(
		diagnostics[0]
			.message
			.contains("there is no component <Sparkle>")
	);

	let diagnostics = ui.check("<Stat label=\"x\" value={1} colour=\"red\" />\n");
	assert!(diagnostics[0].message.contains("has no prop `colour`"));

	let diagnostics = ui.check("<Stat value={1} />\n");
	assert!(diagnostics[0].message.contains("needs `label`"));
}

#[test]
fn documents_cannot_escape_their_box() {
	let ui = AgentUi::new();
	let diagnostics = ui.check("<Card class=\"fixed inset-0 z-50\" title=\"Confirm\" />\n");
	assert!(diagnostics[0].message.contains("not allowed"));

	let diagnostics = ui.check("<div style=\"position: fixed\">Pay now</div>\n");
	assert_ne!(diagnostics, [] as [leptoskit_agent_ui::Diagnostic; 0]);
}

#[test]
fn images_need_the_applications_permission() {
	let blocked = AgentUi::new();
	let diagnostics = blocked.check("![chart](https://tracker.example/pixel.png)\n");
	assert!(diagnostics[0].message.contains("not shown"));

	let allowed = AgentUi::new().allow_images(|url| url.starts_with("https://cdn.example/"));
	assert_eq!(
		allowed.check("![chart](https://cdn.example/a.png)\n"),
		[] as [leptoskit_agent_ui::Diagnostic; 0]
	);
}

#[test]
fn documents_cannot_write_application_data() {
	let ui = AgentUi::new().data("balance", 1000);
	let diagnostics = ui.check(
		"<Var name=\"balance\" value={1000000} />\n\n<Button set=\"balance\" \
		 to={0}>Zero</Button>\n",
	);
	assert!(
		diagnostics
			.iter()
			.any(|diagnostic| diagnostic.message.contains("application's data"))
	);
	assert_eq!(ui.value("balance"), Some(Value::Number(1000.0)));
}

#[test]
fn limits_hold() {
	let ui = AgentUi::new().limits(Limits {
		max_nodes: 20,
		..Limits::default()
	});
	let source = "Line.\n\n".repeat(50);
	let rendered = ui.render(&source);
	assert!(
		rendered
			.diagnostics
			.iter()
			.any(|diagnostic| diagnostic.message.contains("limited to 20 elements"))
	);

	let ui = AgentUi::new().limits(Limits {
		max_source_bytes: 10,
		..Limits::default()
	});
	assert_ne!(
		ui.check("A longer document than ten bytes."),
		[] as [leptoskit_agent_ui::Diagnostic; 0]
	);

	let ui = AgentUi::new();
	let diagnostics = ui.check(
		"{sum(range(1000)) + sum(range(1000)) + sum(range(1000)) + sum(range(1000)) + \
		 sum(range(1000)) + sum(range(1000)) + sum(range(1000)) + sum(range(1000)) + \
		 sum(range(1000)) + sum(range(1000)) + sum(range(1000)) + sum(range(1000)) + \
		 sum(range(1000)) + sum(range(1000)) + sum(range(1000)) + sum(range(1000)) + \
		 sum(range(1000)) + sum(range(1000)) + sum(range(1000)) + sum(range(1000)) + \
		 sum(range(1000))}\n",
	);
	assert!(
		diagnostics
			.iter()
			.any(|diagnostic| diagnostic.message.contains("too much work"))
	);
}

#[test]
fn streaming_documents_render_what_has_arrived() {
	let ui = AgentUi::new();
	let rendered = ui.render("Here is your plan.\n\n<Card title=\"Rent\">\n\nKeep it under");
	assert!(rendered.partial);
	assert!(
		rendered.diagnostics.is_empty(),
		"{:?}",
		rendered.diagnostics
	);

	let ui_for_view = ui.clone();
	let tester = WidgetTester::new(
		move || {
			ui_for_view
				.render("Here is your plan.\n\n<Card title=\"Rent\">\n\nKeep it under")
				.element
		},
		390.0,
		800.0,
	)
	.unwrap();
	tester.get_by_text("Rent").unwrap();
	tester.get_by_text("Keep it under").unwrap();
}

#[test]
fn user_input_survives_the_document_arriving_again() {
	let ui = AgentUi::new();
	let first = "<Var name=\"rent\" value={1800} />\n\n<Stepper bind=\"rent\" step={100} />\n";
	let mut tester = mount(&ui, first);
	tester.tap(&Finder::label("Increase by 100")).unwrap();
	assert_eq!(ui.inputs().get("rent"), Some(&Value::Number(1900.0)));

	// The agent sends the same document with more written.
	ui.render(
		"<Var name=\"rent\" value={1800} />\n\n<Stepper bind=\"rent\" step={100} />\n\nMore.\n",
	);
	assert_eq!(ui.inputs().get("rent"), Some(&Value::Number(1900.0)));

	// A new starting value is the agent changing its mind.
	ui.render("<Var name=\"rent\" value={1500} />\n");
	assert_eq!(ui.inputs().get("rent"), Some(&Value::Number(1500.0)));
}

#[test]
fn applications_add_their_own_components() {
	let ui = AgentUi::new().component(
		Component::new(
			"Envelope",
			"A spending envelope: how much of a category's budget is spent.",
			|props, _cx| {
				Ok(Text(format!(
					"{}: {} of {}",
					props.text("name")?,
					props.number("spent")?,
					props.number("limit")?
				)))
			},
		)
		.prop(Prop::text("name", "The category.").required())
		.prop(Prop::number("spent", "Spent so far.").required())
		.prop(Prop::number("limit", "The budget.").required()),
	);

	let tester = mount(
		&ui,
		"<Var name=\"spent\" value={412} />\n\n<Envelope name=\"Groceries\" spent={spent} \
		 limit={600} />\n",
	);
	tester.get_by_text("Groceries: 412 of 600").unwrap();
	assert!(ui.reference().contains("### `<Envelope>`"));
}

#[test]
fn the_reference_teaches_the_language() {
	let ui = AgentUi::new().data("income", 4200).data(
		"transactions",
		vec![Value::Record(
			[
				("payee".to_owned(), Value::from("Grocer")),
				("amount".to_owned(), Value::from(42.5)),
			]
			.into_iter()
			.collect(),
		)],
	);
	insta::assert_snapshot!(ui.reference());
}

#[test]
fn the_reference_example_renders_cleanly() {
	let ui = AgentUi::new().on_action(|_| {});
	let reference = ui.reference();
	let example = reference
		.split("## Example\n\n```mdx\n")
		.nth(1)
		.and_then(|rest| rest.split("\n```").next())
		.unwrap();
	let rendered = ui.render(example);
	assert!(
		rendered.diagnostics.is_empty(),
		"{:?}",
		rendered.diagnostics
	);

	let ui_for_view = ui.clone();
	let example = example.to_owned();
	let tester =
		WidgetTester::new(move || ui_for_view.render(&example).element, 390.0, 2000.0).unwrap();
	insta::assert_snapshot!(tester.semantics());
}

#[test]
fn components_written_on_one_line_keep_their_text() {
	let ui = AgentUi::new();
	let tester = mount(
		&ui,
		"<Var name=\"left\" value={-40} />\n\n<Callout kind=\"warning\">You would overspend by \
		 **{money(-left)}**.</Callout>\n",
	);
	tester.get_by_text("Warning").unwrap();
	tester.get_by_text("You would overspend by $40.").unwrap();
}

#[test]
fn charts_stay_the_height_they_ask_for() {
	let ui = AgentUi::new();
	for kind in ["bar", "line"] {
		let source: &'static str = Box::leak(
			format!(
				"<Chart type=\"{kind}\" title=\"Spending\" labels={{[\"Rent\", \"Food\"]}} \
				 values={{[1800, -600]}} height={{120}} />\n"
			)
			.into_boxed_str(),
		);
		let tester = mount(&ui, source);
		let chart = tester
			.get_by_label("Spending. Rent 1,800, Food -600")
			.unwrap();
		assert!(chart.frame.height < 200.0, "{kind}: {:?}", chart.frame);
		let label = tester.get_by_text("Rent").unwrap();
		assert!(label.frame.y < 200.0, "{kind}: {:?}", label.frame);
	}
}

#[test]
fn charts_draw_without_vectors_where_a_host_cannot() {
	let ui = AgentUi::new().widgets(Widgets::Basic);
	for kind in ["line", "area", "pie", "donut"] {
		let source: &'static str = Box::leak(
			format!(
				"<Chart type=\"{kind}\" title=\"Spending\" labels={{[\"Rent\", \"Food\"]}} \
				 values={{[1800, 600]}} />\n"
			)
			.into_boxed_str(),
		);
		let tester = mount(&ui, source);
		assert!(
			!tester.tree().contains("vector"),
			"{kind} drew a vector:\n{}",
			tester.tree()
		);
		tester.get_by_text("Rent").unwrap();
	}
}

/// Every widget on screen, parents before children.
fn widgets(tester: &WidgetTester) -> Vec<NodeId> {
	let surface = tester.surface();
	let mut out = Vec::new();
	let mut pending: Vec<NodeId> = surface.root().into_iter().collect();
	while let Some(id) = pending.pop() {
		out.push(id);
		pending.extend(surface.children(id).iter().rev().copied());
	}
	out
}

fn kinds(tester: &WidgetTester) -> Vec<NativeKind> {
	widgets(tester)
		.into_iter()
		.map(|id| tester.surface().kind(id))
		.collect()
}

fn backgrounds(tester: &WidgetTester) -> Vec<Color> {
	widgets(tester)
		.into_iter()
		.filter_map(|id| tester.surface().props(id).background)
		.collect()
}

const fn rgb(value: u32) -> Color {
	Color::rgba((value >> 16) as u8, (value >> 8) as u8, value as u8, 255)
}

/// Variables and data the components' examples read.
const EXAMPLE_SETUP: &str = "<Var name=\"rent\" value={1800} />\n<Var name=\"saving\" value={500} \
                             />\n<Var name=\"plan\" value=\"steady\" />\n<Var name=\"round_up\" \
                             value={false} />\n<Let name=\"left\" value={4200 - rent} />\n<Let \
                             name=\"yearly\" value={rent * 12} />\n\n";

#[test]
fn every_component_example_renders_cleanly_on_every_host() {
	for widgets in [Widgets::All, Widgets::WithoutControls, Widgets::Basic] {
		let ui = AgentUi::new()
			.widgets(widgets)
			.data(
				"transactions",
				vec![std::collections::BTreeMap::from([
					("date".to_owned(), Value::from("May 2")),
					("payee".to_owned(), Value::from("Grocer")),
					("amount".to_owned(), Value::from(-82.4)),
				])],
			)
			.on_action(|_| {});
		let reference = ui.reference();
		let examples: Vec<&str> = reference
			.split("```mdx\n")
			.skip(1)
			.filter_map(|block| block.split("\n```").next())
			.collect();
		assert!(examples.len() >= 17, "{} examples", examples.len());

		for example in examples {
			let source: &'static str =
				Box::leak(format!("{EXAMPLE_SETUP}{example}\n").into_boxed_str());
			let rendered = ui.render(source);
			assert!(
				rendered.diagnostics.is_empty(),
				"{widgets:?}: {example}\n{:?}",
				rendered.diagnostics
			);
			// Mounting commits the tree, which the host checks for widgets it
			// cannot nest.
			let tester = mount(&ui, source);
			let kinds = kinds(&tester);
			let native_controls = [
				NativeKind::Slider,
				NativeKind::Switch,
				NativeKind::Checkbox,
				NativeKind::Radio,
				NativeKind::ActivityIndicator,
			];
			if widgets != Widgets::All {
				assert!(
					!kinds.iter().any(|kind| native_controls.contains(kind)),
					"{widgets:?} drew a native control for {example}:\n{}",
					tester.tree()
				);
			}
			if widgets == Widgets::Basic {
				assert!(
					!kinds
						.iter()
						.any(|kind| matches!(kind, NativeKind::Vector | NativeKind::Pressable)),
					"Basic drew a vector or pressable for {example}:\n{}",
					tester.tree()
				);
			}
		}
	}
}

#[test]
fn a_native_slider_moves_a_variable_in_place() {
	let ui = AgentUi::new();
	let mut tester = mount(
		&ui,
		"<Var name=\"saving\" value={500} />\n\n<Slider bind=\"saving\" min={0} max={2000} \
		 step={50} label=\"Monthly saving\" format=\"money\" />\n\nA year: {money(saving * 12)}\n",
	);
	let slider = Finder::label("Monthly saving");
	let before = tester.find(&slider).unwrap();
	assert_eq!(before.kind, NativeKind::Slider);
	let id = |tester: &WidgetTester| {
		widgets(tester)
			.into_iter()
			.find(|id| tester.surface().kind(*id) == NativeKind::Slider)
			.unwrap()
	};
	let first = id(&tester);

	tester.set_slider_value(&slider, 760.0).unwrap();
	tester.get_by_text("$750").unwrap();
	tester.get_by_text("A year: $9,000").unwrap();
	assert_eq!(ui.value("saving"), Some(Value::Number(750.0)));
	// The same native slider, updated rather than replaced, so a drag in
	// progress keeps going.
	assert_eq!(id(&tester), first);
	assert_eq!(
		tester
			.find(&slider)
			.unwrap()
			.props
			.slider
			.map(leptoskit_native::SliderState::value),
		Some(750.0)
	);
}

#[test]
fn a_slider_without_native_controls_is_a_track_of_stops() {
	let ui = AgentUi::new().widgets(Widgets::WithoutControls);
	let mut tester = mount(
		&ui,
		"<Var name=\"saving\" value={500} />\n\n<Slider bind=\"saving\" min={0} max={2000} \
		 step={50} label=\"Monthly saving\" format=\"money\" />\n",
	);
	assert!(!kinds(&tester).contains(&NativeKind::Slider));

	tester
		.tap(&Finder::label("Set Monthly saving to $1,000"))
		.unwrap();
	tester.get_by_text("$1,000").unwrap();
	tester
		.tap(&Finder::label("Increase Monthly saving by $50"))
		.unwrap();
	tester.get_by_text("$1,050").unwrap();
	tester
		.tap(&Finder::label("Set Monthly saving to $2,000"))
		.unwrap();
	assert_eq!(
		tester
			.find(&Finder::label("Increase Monthly saving by $50"))
			.unwrap()
			.props
			.enabled,
		Some(false)
	);
}

#[test]
fn switches_checkboxes_and_radios_are_native() {
	let ui = AgentUi::new();
	let mut tester = mount(
		&ui,
		"<Var name=\"round_up\" value={false} />\n<Var name=\"alerts\" value={false} />\n<Var \
		 name=\"plan\" value=\"steady\" />\n\n<Toggle bind=\"round_up\" label=\"Round up\" \
		 />\n\n<Toggle bind=\"alerts\" kind=\"checkbox\" label=\"Alerts\" />\n\n<Choice \
		 bind=\"plan\" kind=\"radio\" options={[\"steady\", \"fast\"]} labels={[\"Steady\", \
		 \"Fast\"]} />\n\n{round_up ? \"Rounding\" : \"Exact\"}, {alerts}, {plan}\n",
	);
	assert_eq!(
		tester.find(&Finder::label("Round up")).unwrap().kind,
		NativeKind::Switch
	);
	tester.toggle(&Finder::label("Round up"), true).unwrap();
	tester.toggle(&Finder::label("Alerts"), true).unwrap();
	tester.toggle(&Finder::label("Fast"), true).unwrap();
	tester.get_by_text("Rounding, true, fast").unwrap();
	assert_eq!(
		tester.find(&Finder::label("Fast")).unwrap().props.checked,
		Some(true)
	);
	assert_eq!(
		tester.find(&Finder::label("Steady")).unwrap().props.checked,
		Some(false)
	);
}

#[test]
fn a_pressable_sends_its_action_and_holds_no_controls() {
	let actions = Arc::new(Mutex::new(Vec::new()));
	let seen = Arc::clone(&actions);
	let ui = AgentUi::new().on_action(move |action| seen.lock().unwrap().push(action.clone()));
	let mut tester = mount(
		&ui,
		"<Pressable label=\"Choose steady\" action=\"choose\" value=\"steady\" class=\"p-4 \
		 rounded-xl bg-surface\">\n  <Text class=\"font-semibold\">Steady</Text>\n  <Text \
		 class=\"text-sm text-muted\">$400 a month</Text>\n</Pressable>\n",
	);
	assert_eq!(
		tester.find(&Finder::label("Choose steady")).unwrap().kind,
		NativeKind::Pressable
	);
	tester.tap(&Finder::label("Choose steady")).unwrap();
	let actions = actions.lock().unwrap();
	assert_eq!(actions.len(), 1);
	assert_eq!(actions[0].name, "choose");
	assert_eq!(actions[0].value, Value::from("steady"));

	let rendered = ui.render(
		"<Pressable label=\"Outer\" action=\"a\">\n  <Button \
		 action=\"b\">Inner</Button>\n</Pressable>\n",
	);
	assert!(
		rendered.diagnostics[0]
			.message
			.contains("cannot hold <Button>"),
		"{:?}",
		rendered.diagnostics
	);
}

#[test]
fn colours_follow_the_application_theme() {
	let ui = AgentUi::new();
	let source = "<Card title=\"May\">\n\nOn track.\n\n</Card>\n";

	leptoskit_native::Theme::clear();
	let tester = mount(&ui, source);
	assert!(
		backgrounds(&tester).contains(&Color::rgba(128, 128, 128, 24)),
		"{:?}",
		backgrounds(&tester)
	);

	// An application that themes `surface` sees its own colour, with no
	// change to the document or the renderer.
	leptoskit_native::Theme::set_color("surface", rgb(0x00f4_efe6));
	let tester = mount(&ui, source);
	assert!(backgrounds(&tester).contains(&rgb(0x00f4_efe6)));
	leptoskit_native::Theme::clear();
}

#[test]
fn documents_style_with_roles_and_formulas() {
	let ui = AgentUi::new().theme(leptoskit_agent_ui::Theme::default().color(
		"savings",
		rgb(0x000f_766e),
		rgb(0x002d_d4bf),
		"Money set aside.",
	));
	leptoskit_native::Theme::clear();
	let mut tester = mount(
		&ui,
		"<Var name=\"left\" value={100} />\n\n<View class={left < 0 ? \"p-2 bg-negative\" : \"p-2 \
		 bg-savings\"}>\n  <Text>Balance</Text>\n</View>\n\n<Button set=\"left\" \
		 to={-5}>Spend</Button>\n",
	);
	assert!(backgrounds(&tester).contains(&rgb(0x000f_766e)));
	assert!(ui.reference().contains("`savings`"));

	tester.tap(&button("Spend")).unwrap();
	assert!(backgrounds(&tester).contains(&rgb(0x00d0_3b3b)));
	assert!(!backgrounds(&tester).contains(&rgb(0x000f_766e)));

	let refused = ui.render("<View class={\"fixed inset-0\"}>x</View>\n");
	assert!(
		refused.diagnostics[0].message.contains("not allowed"),
		"{:?}",
		refused.diagnostics
	);
}
