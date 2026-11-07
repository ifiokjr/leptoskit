//! The built-in components, drawn and pressed.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;

use leptoskit_agent_ui::Action;
use leptoskit_agent_ui::AgentUi;
use leptoskit_agent_ui::Component;
use leptoskit_agent_ui::Context;
use leptoskit_agent_ui::Prop;
use leptoskit_agent_ui::Props;
use leptoskit_agent_ui::Value;
use leptoskit_agent_ui::Widgets;
use leptoskit_native::NativeKind;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_testing::Finder;
use leptoskit_testing::Queries;
use leptoskit_testing::WidgetTester;

fn mount(ui: &AgentUi, source: &'static str) -> WidgetTester {
	let ui = ui.clone();
	WidgetTester::new(move || ui.render(source).element, 390.0, 3000.0).unwrap()
}

fn diagnostics(ui: &AgentUi, source: &str) -> Vec<String> {
	ui.check(source)
		.into_iter()
		.map(|diagnostic| diagnostic.message)
		.collect()
}

fn assert_clean(ui: &AgentUi, source: &str) {
	let messages = diagnostics(ui, source);
	assert!(messages.is_empty(), "{source}\n{messages:?}");
}

#[test]
fn layouts_space_and_align_their_content() {
	use std::fmt::Write as _;

	let ui = AgentUi::new();
	let mut source = String::from("<Var name=\"x\" value={1} />\n\n");
	for gap in [0, 1, 2, 3, 4, 5, 6, 7, 8, 20] {
		writeln!(
			source,
			"<Stack gap={{{gap}}}><Text>gap {gap}</Text></Stack>"
		)
		.unwrap();
	}
	for align in ["start", "center", "end", "stretch", "else"] {
		writeln!(
			source,
			"<Row align=\"{align}\" justify=\"start\"><Text>{align}</Text></Row>"
		)
		.unwrap();
	}
	for justify in ["start", "center", "end", "between", "else"] {
		writeln!(
			source,
			"<Row justify=\"{justify}\"><Text>{justify}</Text></Row>"
		)
		.unwrap();
	}
	source.push_str(
		"<Row wrap={false} equal><Text>fixed</Text><Text>widths</Text></Row>\n\n<Row \
		 equal><Text>equal</Text><Text>items</Text></Row>\n",
	);
	let source: &'static str = Box::leak(source.into_boxed_str());
	assert_clean(&ui, source);
	let tester = mount(&ui, source);
	tester.get_by_text("gap 0").unwrap();
	tester.get_by_text("fixed").unwrap();
}

#[test]
fn cards_and_callouts_group_their_content() {
	let ui = AgentUi::new();
	let source = "<Card title=\"This month\" subtitle=\"A quieter line\">\n\nOn \
	              track.\n\n</Card>\n\n<Card>\n\nNo heading.\n\n</Card>\n\n<Callout \
	              kind=\"tip\">Tip.</Callout>\n\n<Callout \
	              kind=\"warning\">Warning.</Callout>\n\n<Callout kind=\"danger\" \
	              title=\"Careful\">Danger.</Callout>\n\n<Callout \
	              kind=\"note\">Note.</Callout>\n\n<Callout>Default.</Callout>\n";
	assert_clean(&ui, source);
	let tester = mount(&ui, source);
	tester.get_by_text("This month").unwrap();
	tester.get_by_text("A quieter line").unwrap();
	tester.get_by_text("Tip").unwrap();
	tester.get_by_text("Warning").unwrap();
	tester.get_by_text("Careful").unwrap();
	assert!(tester.get_all_by_text("Note").unwrap().len() >= 2);
}

#[test]
fn dividers_and_conditional_content() {
	let ui = AgentUi::new();
	let source = "<Var name=\"left\" value={-1} />\n\n<Divider />\n\n<Show when={left < \
	              0}>\n\nOver budget.\n\n</Show>\n\n<Show when={left > 0}>\n\nUnder \
	              budget.\n\n</Show>\n";
	assert_clean(&ui, source);
	let tester = mount(&ui, source);
	tester.get_by_text("Over budget.").unwrap();
	assert!(tester.query_by_text("Under budget.").unwrap().is_none());
}

#[test]
fn stats_show_their_change_in_both_directions() {
	let ui = AgentUi::new();
	let source = "<Stat label=\"Spent\" value={412} format=\"money\" delta={-120} />\n\n<Stat \
	              label=\"Saved\" value={412} delta={120} />\n\n<Stat label=\"Cents\" value={1.5} \
	              />\n\n<Stat label=\"Share\" value={0.25} format=\"percent\" />\n\n<Stat \
	              label=\"Text\" value=\"a lot\" />\n\n<Stat label=\"Down\" value={100} \
	              delta={-5} good=\"down\" caption=\"Lower is better\" />\n\n<Stat \
	              label=\"Falling text\" value={100} delta=\"-4% vs May\" />\n\n<Stat \
	              label=\"Rising text\" value={100} delta=\"+4% vs May\" />\n\n<Stat label=\"Flat \
	              text\" value={100} delta=\"steady\" />\n";
	assert_clean(&ui, source);
	let tester = mount(&ui, source);
	tester.get_by_text("▼ -$120").unwrap();
	tester.get_by_text("▲ +120").unwrap();
	tester.get_by_text("1.50").unwrap();
	tester.get_by_text("25%").unwrap();
	tester.get_by_text("a lot").unwrap();
	tester.get_by_text("Lower is better").unwrap();
	tester.get_by_text("▼ -4% vs May").unwrap();
	tester.get_by_text("▲ +4% vs May").unwrap();
}

#[test]
fn progress_meters_fill_and_overflow() {
	let ui = AgentUi::new();
	let source = "<Progress label=\"Groceries\" value={412} max={600} format=\"money\" \
	              />\n\n<Progress value={120} max={100} />\n\n<Progress value={50} \
	              tone=\"warning\" />\n\n<Progress value={-10} />\n";
	assert_clean(&ui, source);
	let tester = mount(&ui, source);
	tester.get_by_label("Groceries: $412 of $600").unwrap();
	tester.get_by_label("Progress: 120 of 100").unwrap();
	tester.get_by_label("Progress: 50 of 100").unwrap();

	let messages = diagnostics(&ui, "<Progress value={1} max={0} />\n");
	assert!(
		messages[0].contains("`max` must be more than 0"),
		"{messages:?}"
	);
}

#[test]
fn spinners_follow_the_host() {
	let ui = AgentUi::new();
	assert_clean(&ui, "<Spinner />\n<Spinner label=\"Working\" />\n");
	let tester = mount(&ui, "<Spinner label=\"Working\" />\n");
	assert!(
		tester
			.surface()
			.root()
			.map(|root| tester.surface().kind(root))
			.is_some()
	);

	let plain = AgentUi::new().widgets(Widgets::WithoutControls);
	let source = "<Spinner />\n\n<Spinner label=\"Working\" />\n";
	assert_clean(&plain, source);
	let tester = mount(&plain, source);
	tester.get_by_text("Loading…").unwrap();
	tester.get_by_text("Working…").unwrap();
}

fn transactions(count: usize) -> Vec<BTreeMap<String, Value>> {
	(0..count)
		.map(|index| {
			BTreeMap::from([
				("date".to_owned(), Value::from(format!("May {}", index + 1))),
				("payee".to_owned(), Value::from("Grocer")),
				("amount".to_owned(), Value::from(-82.4)),
			])
		})
		.collect()
}

#[test]
fn data_tables_read_their_columns_or_infer_them() {
	let ui = AgentUi::new().data("rows", transactions(3));
	let source = "<DataTable rows={rows} />\n";
	assert_clean(&ui, source);
	let tester = mount(&ui, source);
	tester.get_by_text("date").unwrap();
	tester.get_by_text("payee").unwrap();

	let source = "<DataTable rows={rows} columns={[\"payee\", \"missing\"]} headers={[\"Who\"]} \
	              money={[\"missing\"]} />\n";
	assert_clean(&ui, source);
	let tester = mount(&ui, source);
	tester.get_by_text("Who").unwrap();
	assert!(tester.get_all_by_text("Grocer").unwrap().len() >= 3);
}

#[test]
fn data_tables_cap_their_rows() {
	let ui = AgentUi::new().data("rows", transactions(205));
	let source = "<DataTable rows={rows} columns={[\"date\"]} />\n";
	assert_clean(&ui, source);
	let ui_for_view = ui.clone();
	let tester =
		WidgetTester::new(move || ui_for_view.render(source).element, 390.0, 8000.0).unwrap();
	tester.get_by_text("5 more rows not shown").unwrap();
}

#[test]
fn data_tables_report_rows_they_cannot_read() {
	let ui = AgentUi::new().data("rows", vec![Value::Number(1.0)]);
	let messages = diagnostics(&ui, "<DataTable rows={rows} />\n");
	assert!(
		messages[0].contains("`columns` is required"),
		"{messages:?}"
	);

	let source = "<DataTable rows={rows} columns={[\"date\"]} />\n";
	let rendered = ui.render(source);
	assert!(
		rendered.diagnostics.is_empty(),
		"{:?}",
		rendered.diagnostics
	);
	let tester = mount(&ui, source);
	assert_ne!(tester.tree(), "");
}

#[test]
fn buttons_send_either_a_variable_a_action_or_both() {
	let actions = Arc::new(Mutex::new(Vec::<Action>::new()));
	let ui = AgentUi::new().on_action({
		let actions = Arc::clone(&actions);
		move |action| actions.lock().unwrap().push(action.clone())
	});
	let mut tester = mount(
		&ui,
		"<Var name=\"plan\" value=\"steady\" />\n\n<Button label=\"Label prop\" action=\"go\" \
		 variant=\"secondary\" />\n\n<Button action=\"apply\" value={2} set=\"plan\" \
		 to=\"fast\">Apply</Button>\n\n<Button set=\"plan\" to=\"other\">Switch</Button>\n",
	);
	tester.get_by_text("Label prop").unwrap();
	tester
		.tap(&Finder::role(NativeKind::Button, "Apply"))
		.unwrap();
	tester
		.tap(&Finder::role(NativeKind::Button, "Switch"))
		.unwrap();
	assert_eq!(
		actions
			.lock()
			.unwrap()
			.last()
			.map(|action| action.name.as_str()),
		Some("apply")
	);

	let ui = AgentUi::new();
	let messages = diagnostics(&ui, "<Button action=\"a\" />\n");
	assert!(messages[0].contains("needs a label"), "{messages:?}");
}

#[test]
fn pressables_hold_their_content_or_fall_back_to_a_button() {
	let ui = AgentUi::new().on_action(|_| {});
	let source = "<Pressable label=\"Choose steady\" action=\"choose\" value=\"steady\" \
	              class=\"rounded-xl p-4 gap-1 bg-surface\">\n\n<Text \
	              class=\"font-semibold\">Steady</Text>\n\n<Text class=\"text-sm text-muted\">A \
	              plan</Text>\n\n</Pressable>\n";
	assert_clean(&ui, source);
	let tester = mount(&ui, source);
	tester.get_by_text("Steady").unwrap();

	let plain = AgentUi::new().widgets(Widgets::Basic);
	let tester = mount(
		&plain,
		"<Pressable label=\"Choose steady\" action=\"choose\">\n\nSteady\n\n</Pressable>\n",
	);
	assert_eq!(
		tester
			.find(&Finder::role(NativeKind::Button, "Choose steady"))
			.unwrap()
			.kind,
		NativeKind::Button
	);

	let messages = diagnostics(&ui, "<Pressable label=\" \">x</Pressable>\n");
	assert!(messages[0].contains("needs a `label`"), "{messages:?}");
}

#[test]
fn sliders_validate_their_range_and_step() {
	let ui = AgentUi::new();
	let messages = diagnostics(
		&ui,
		"<Var name=\"x\" value={1} />\n<Slider bind=\"x\" min={10} max={0} />\n",
	);
	assert!(
		messages[0].contains("`max` must be more than `min`"),
		"{messages:?}"
	);

	let messages = diagnostics(
		&ui,
		"<Var name=\"x\" value={1} />\n<Slider bind=\"x\" min={0} max={10} step={0} />\n",
	);
	assert!(
		messages[0].contains("`step` must be more than 0"),
		"{messages:?}"
	);

	let messages = diagnostics(
		&ui,
		"<Var name=\"x\" value={1} />\n<Slider bind=\"x\" min={0} max={10} step={20} />\n",
	);
	assert!(
		messages[0].contains("`step` must be more than 0"),
		"{messages:?}"
	);

	let messages = diagnostics(
		&ui,
		"<Var name=\"x\" value={1000000} />\n<Slider bind=\"x\" min={1000000} max={2000000} \
		 step={0.000000000001} />\n",
	);
	assert!(
		messages[0].contains("cannot slide over that range"),
		"{messages:?}"
	);

	let source = "<Var name=\"x\" value={500000} />\n<Slider bind=\"x\" min={1000000} \
	              max={2000000} step={0.0000001} />\n";
	let rendered = ui.render(source);
	assert!(
		rendered.diagnostics.is_empty(),
		"{:?}",
		rendered.diagnostics
	);
}

#[test]
fn a_variable_prop_can_be_written_as_a_bare_expression() {
	let ui = AgentUi::new();
	let mut tester = mount(
		&ui,
		"<Var name=\"x\" value={5} />\n\n<Slider bind={x} min={0} max={10} />\n",
	);
	tester
		.set_slider_value(&Finder::label("Value"), 8.0)
		.unwrap();
	assert_eq!(ui.inputs().get("x"), Some(&Value::Number(8.0)));
}

#[test]
fn sliders_without_a_step_move_freely() {
	let ui = AgentUi::new();
	let mut tester = mount(
		&ui,
		"<Var name=\"x\" value={5} />\n\n<Slider bind=\"x\" min={0} max={10} />\n\n{x}\n",
	);
	tester
		.set_slider_value(&Finder::label("Value"), 7.5)
		.unwrap();
	tester.get_by_text("7.5").unwrap();
}

#[test]
fn a_slider_without_native_controls_has_stops_and_no_step() {
	let ui = AgentUi::new().widgets(Widgets::WithoutControls);
	let mut tester = mount(
		&ui,
		"<Var name=\"x\" value={0} />\n\n<Slider bind=\"x\" min={0} max={100} />\n",
	);
	tester.tap(&Finder::label("Increase Value by 1")).unwrap();
	tester.get_by_text("1").unwrap();
	tester.tap(&Finder::label("Decrease Value by 1")).unwrap();
	tester.get_by_text("0").unwrap();
}

#[test]
fn steppers_change_a_variable_within_their_bounds() {
	let ui = AgentUi::new();
	let mut tester = mount(
		&ui,
		"<Var name=\"count\" value={1} />\n\n<Stepper bind=\"count\" min={0} max={2} \
		 label=\"Count\" />\n\n{count}\n",
	);
	tester.get_by_text("Count").unwrap();
	tester.tap(&Finder::label("Increase by 1")).unwrap();
	assert_eq!(ui.inputs().get("count"), Some(&Value::Number(2.0)));
	assert_eq!(
		tester
			.find(&Finder::label("Increase by 1"))
			.unwrap()
			.props
			.enabled,
		Some(false)
	);
	tester.tap(&Finder::label("Decrease by 1")).unwrap();
	assert_eq!(ui.inputs().get("count"), Some(&Value::Number(1.0)));
}

#[test]
fn fields_write_numbers_and_text() {
	let ui = AgentUi::new();
	let mut tester = mount(
		&ui,
		"<Var name=\"amount\" value={0} />\n<Var name=\"note\" value=\"\" />\n\n<NumberField \
		 bind=\"amount\" label=\"Amount\" placeholder=\"0\" />\n\n<TextField bind=\"note\" \
		 label=\"Note\" placeholder=\"Anything\" />\n\n<TextField bind=\"note\" placeholder=\"A \
		 note\" />\n\n{amount} {note}\n",
	);
	tester
		.enter_text(&Finder::label("Amount"), "1,800")
		.unwrap();
	tester.enter_text(&Finder::label("Note"), "thanks").unwrap();
	assert_eq!(ui.value("amount"), Some(Value::Number(1800.0)));
	assert_eq!(ui.value("note"), Some(Value::from("thanks")));

	// Text that is not a number leaves the amount alone; empty clears it.
	let before = ui.value("amount");
	tester.enter_text(&Finder::label("Amount"), "abc").unwrap();
	assert_eq!(ui.value("amount"), before);
	tester.enter_text(&Finder::label("Amount"), " ").unwrap();
	assert_eq!(ui.value("amount"), Some(Value::Number(0.0)));
}

#[test]
fn choices_offer_buttons_or_radios() {
	let ui = AgentUi::new();
	let mut tester = mount(
		&ui,
		"<Var name=\"plan\" value=\"steady\" />\n\n<Choice bind=\"plan\" options={[\"steady\", \
		 \"fast\"]} labels={[\"Steady\", \"Fast\"]} label=\"Plan\" />\n\n<Choice bind=\"plan\" \
		 options={[\"steady\", \"fast\"]} />\n\n{plan}\n",
	);
	tester.get_by_text("Plan").unwrap();
	tester
		.tap(&Finder::role(NativeKind::Button, "Fast"))
		.unwrap();
	assert_eq!(ui.inputs().get("plan"), Some(&Value::from("fast")));

	let mut tester = mount(
		&ui,
		"<Var name=\"plan\" value=\"steady\" />\n\n<Choice bind=\"plan\" kind=\"radio\" \
		 options={[\"steady\", \"fast\"]} labels={[\"Steady\", \"Fast\"]} />\n",
	);
	tester.toggle(&Finder::label("Fast"), true).unwrap();
	assert_eq!(ui.inputs().get("plan"), Some(&Value::from("fast")));

	let messages = diagnostics(
		&ui,
		"<Var name=\"plan\" value=\"steady\" />\n<Choice bind=\"plan\" kind=\"radio\" \
		 options={[\"steady\", \"steady\"]} />\n",
	);
	assert!(messages[0].contains("lists steady twice"), "{messages:?}");
}

#[test]
fn choices_without_native_controls_are_buttons() {
	let ui = AgentUi::new().widgets(Widgets::WithoutControls);
	let mut tester = mount(
		&ui,
		"<Var name=\"plan\" value=\"steady\" />\n\n<Choice bind=\"plan\" kind=\"radio\" \
		 options={[\"steady\", \"fast\"]} labels={[\"Steady\", \"Fast\"]} />\n",
	);
	tester
		.tap(&Finder::role(NativeKind::Button, "Fast"))
		.unwrap();
	assert_eq!(ui.inputs().get("plan"), Some(&Value::from("fast")));
}

#[test]
fn toggles_turn_variables_on_and_off() {
	let ui = AgentUi::new();
	let mut tester = mount(
		&ui,
		"<Var name=\"on\" value={false} />\n<Var name=\"checked\" value={false} />\n\n<Toggle \
		 bind=\"on\" label=\"On\" />\n\n<Toggle bind=\"checked\" kind=\"checkbox\" \
		 label=\"Checked\" />\n\n{on} {checked}\n",
	);
	tester.toggle(&Finder::label("On"), true).unwrap();
	tester.toggle(&Finder::label("Checked"), true).unwrap();
	tester.get_by_text("true true").unwrap();

	let plain = AgentUi::new().widgets(Widgets::WithoutControls);
	let mut tester = mount(
		&plain,
		"<Var name=\"on\" value={false} />\n\n<Toggle bind=\"on\" label=\"On\" />\n",
	);
	tester
		.tap(&Finder::role(NativeKind::Button, "☐ On"))
		.unwrap();
	assert_eq!(plain.inputs().get("on"), Some(&Value::Bool(true)));
}

#[test]
fn a_component_can_read_and_change_variables() {
	let seen = Arc::new(Mutex::new(Vec::<String>::new()));
	let ui = AgentUi::new().data("income", 4200).component(
		Component::new("Probe", "Reads variables and props.", {
			let seen = Arc::clone(&seen);
			move |props, cx| {
				let input = cx.input(props, "bind")?;
				let name = input.name().to_owned();
				let current = input.current();
				let read = cx.read("income").unwrap_or_default();
				seen.lock().unwrap().push(format!(
					"{name}: {current} / {read} / {} / {}",
					props.number("n")?,
					cx.text().trim()
				));
				input.set(Value::Number(7.0));
				Ok(View()
					.class(cx.style("bg-surface p-2"))
					.child(Text(format!("{name} is {}", input.get()))))
			}
		})
		.prop(Prop::variable("bind", "The variable to read.").required())
		.prop(Prop::number("n", "A number.").required())
		.children("Echoed text."),
	);
	let tester = mount(
		&ui,
		"<Var name=\"amount\" value={1} />\n\n<Probe bind=\"amount\" n={2}>hello</Probe>\n",
	);
	tester.get_by_text("amount is 7").unwrap();
	assert_eq!(
		seen.lock().unwrap().last().unwrap(),
		"amount: 1 / 4200 / 2 / hello"
	);

	let messages = diagnostics(&ui, "<Probe bind=\"missing\" n={2} />\n");
	assert!(
		messages[0].contains("there is no variable `missing`"),
		"{messages:?}"
	);

	let messages = diagnostics(
		&AgentUi::new().data("income", 1).component(
			Component::new("Probe", "Reads.", |props, cx| {
				let _ = cx.input(props, "bind")?;
				Ok(Text("ok"))
			})
			.prop(Prop::variable("bind", "The variable.").required()),
		),
		"<Probe bind=\"income\" />\n",
	);
	assert!(messages[0].contains("cannot be changed"), "{messages:?}");
}

#[test]
fn an_input_forgets_its_variable_and_refuses_what_is_too_large() {
	let input = Arc::new(Mutex::new(None));
	let ui = AgentUi::new()
		.limits(leptoskit_agent_ui::Limits {
			max_value_bytes: 4,
			..leptoskit_agent_ui::Limits::default()
		})
		.component(
			Component::new("Hold", "Holds its input.", {
				let input = Arc::clone(&input);
				move |props, cx| {
					let held = cx.input(props, "bind")?;
					*input.lock().unwrap() = Some(held);
					Ok(Text("held"))
				}
			})
			.prop(Prop::variable("bind", "The variable.").required()),
		);
	let tester = mount(
		&ui,
		"<Var name=\"x\" value=\"ok\" />\n\n<Hold bind=\"x\" />\n",
	);
	tester.get_by_text("held").unwrap();

	let input = input.lock().unwrap().clone().unwrap();
	assert_eq!(input.name(), "x");
	assert_eq!(input.get(), Value::from("ok"));
	input.set(Value::from("much too long"));
	assert_eq!(input.get(), Value::from("ok"));
	ui.reset();
	assert_eq!(input.get(), Value::Null);
}

#[test]
fn a_component_can_ask_for_an_action_and_reserve_nodes() {
	let actions = Arc::new(Mutex::new(Vec::<String>::new()));
	let ui = AgentUi::new().on_action({
		let actions = Arc::clone(&actions);
		move |action: &Action| actions.lock().unwrap().push(action.name.clone())
	});
	let ui = ui.component(
		Component::new("Send", "Sends an action.", |_props, cx| {
			let send = cx.action("probe", Value::from("v"), "Probe");
			cx.reserve(1)?;
			Ok(
				leptoskit_native::components::Button("Send").on_press(move || {
					if let Some(send) = &send {
						send();
					}
				}),
			)
		})
		.prop(Prop::text("action", "The action name.")),
	);
	let mut tester = mount(&ui, "<Send />\n");
	tester
		.tap(&Finder::role(NativeKind::Button, "Send"))
		.unwrap();
	assert_eq!(
		actions.lock().unwrap().last().map(String::as_str),
		Some("probe")
	);

	let quiet =
		AgentUi::new().component(Component::new("Send", "Sends an action.", |_props, cx| {
			assert!(cx.action("probe", Value::Null, "Probe").is_none());
			Ok(Text("quiet"))
		}));
	let tester = mount(&quiet, "<Send />\n");
	tester.get_by_text("quiet").unwrap();
}

#[test]
fn a_component_that_reserves_more_than_it_has_fails() {
	let ui = AgentUi::new()
		.limits(leptoskit_agent_ui::Limits {
			max_nodes: 2,
			..leptoskit_agent_ui::Limits::default()
		})
		.component(Component::new("Greedy", "Reserves nodes.", |_props, cx| {
			cx.reserve(1000)?;
			Ok(Text("never"))
		}));
	let messages = diagnostics(&ui, "<Greedy />\n");
	assert!(
		messages[0].contains("limited to 2 elements"),
		"{messages:?}"
	);
}

#[test]
fn props_read_the_values_they_were_given() {
	let observed = Arc::new(Mutex::new(Vec::<String>::new()));
	let ui = AgentUi::new().component(
		Component::new("Probe", "Probes props.", {
			let observed = Arc::clone(&observed);
			move |props: &Props, _cx: &mut Context| {
				let mut lines = Vec::new();
				lines.push(format!("{:?}", props.number("n")));
				lines.push(format!("{:?}", props.number("missing")));
				lines.push(format!("{:?}", props.number("s")));
				lines.push(format!("{:?}", props.number_or("n", 3.0)));
				lines.push(format!("{:?}", props.number_or("missing", 4.0)));
				lines.push(format!("{:?}", props.text("missing")));
				lines.push(format!("{:?}", props.text_or("s", "default")));
				lines.push(format!("{:?}", props.bool_or("b", true)));
				lines.push(format!("{:?}", props.bool_or("s", true)));
				lines.push(format!("{:?}", props.list("missing")));
				lines.push(format!("{:?}", props.numbers("missing")));
				lines.push(format!("{:?}", props.numbers("mixed")));
				lines.push(format!("{:?}", props.texts("missing")));
				observed.lock().unwrap().push(lines.join("\n"));
				Ok(Text("probed"))
			}
		})
		.prop(Prop::number("n", "A number."))
		.prop(Prop::text("s", "Some text."))
		.prop(Prop::list("mixed", "A list.")),
	);

	let rendered = ui.render("<Probe n={1} s=\"false\" mixed={[1, \"a\"]} />\n");
	assert!(
		rendered.diagnostics.is_empty(),
		"{:?}",
		rendered.diagnostics
	);
	let observed = observed.lock().unwrap();
	let lines: Vec<&str> = observed.last().unwrap().lines().collect();
	assert_eq!(lines[0], "Ok(1.0)");
	assert_eq!(lines[1], "Err(\"`missing` is required\")");
	assert_eq!(lines[2], "Err(\"`s` must be a number, not text\")");
	assert_eq!(lines[3], "Ok(1.0)");
	assert_eq!(lines[4], "Ok(4.0)");
	assert_eq!(lines[5], "Err(\"`missing` is required\")");
	assert_eq!(lines[6], "\"false\"");
	assert_eq!(lines[7], "true");
	assert_eq!(lines[8], "false");
	assert!(lines[9].starts_with("Err("));
	assert!(lines[10].starts_with("Err("));
	assert!(lines[11].contains("must hold numbers"));
	assert_eq!(lines[12], "Ok([])");
}
