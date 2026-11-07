//! Documents rendered as data: Markdown, components, declarations, and the
//! problems a document can have.

use leptoskit_agent_ui::AgentUi;
use leptoskit_agent_ui::Component;
use leptoskit_agent_ui::Diagnostic;
use leptoskit_agent_ui::Limits;
use leptoskit_agent_ui::Prop;
use leptoskit_agent_ui::Value;
use leptoskit_agent_ui::Widgets;
use leptoskit_native::components::Text;
use leptoskit_testing::Queries;
use leptoskit_testing::WidgetTester;

fn mount(ui: &AgentUi, source: &'static str) -> WidgetTester {
	let ui = ui.clone();
	WidgetTester::new(move || ui.render(source).element, 390.0, 2000.0).unwrap()
}

fn messages(ui: &AgentUi, source: &str) -> Vec<String> {
	ui.check(source)
		.into_iter()
		.map(|diagnostic| diagnostic.message)
		.collect()
}

fn one(ui: &AgentUi, source: &str) -> String {
	let mut messages = messages(ui, source);
	assert_eq!(messages.len(), 1, "{messages:?}");
	messages.remove(0)
}

#[test]
fn every_markdown_construct_draws() {
	let ui = AgentUi::new();
	let source = r"# One

## Two

### Three

#### Four

A paragraph with **bold**, *italic*, `code`, ~~struck~~, a [link](https://example.com), and an
image ![chart](https://example.com/pixel.png) among it.

- bullet
- [x] done
- [ ] todo

1. first
2. second

> A quote
>
> With two blocks

```rust
let x = 1;
```

---

| Left | Right |
| :--- | ---: |
| a | b |

A line with a break\
and the text after it.
";
	let rendered = ui.render(source);
	assert!(
		rendered.diagnostics.is_empty(),
		"{:?}",
		rendered.diagnostics
	);

	let tester = mount(&ui, source);
	tester.get_by_text("One").unwrap();
	tester.get_by_text("With two blocks").unwrap();
	tester.get_by_text("let x = 1;").unwrap();
	assert!(
		tester.tree().contains("A line with a break"),
		"{}",
		tester.tree()
	);
	assert!(
		tester.tree().contains("and the text after it."),
		"{}",
		tester.tree()
	);
	tester.get_by_text("☑").unwrap();
	tester.get_by_text("☐").unwrap();
	tester.get_by_text("1.").unwrap();
	tester.get_by_text("2.").unwrap();
}

#[test]
fn a_table_renders_its_cells() {
	let ui = AgentUi::new();
	let tester = mount(
		&ui,
		"| Category | Spent |\n| --- | ---: |\n| Rent | 1800 |\n| Food | 600 |\n",
	);
	tester.get_by_text("Category").unwrap();
	tester.get_by_text("Spent").unwrap();
	tester.get_by_text("Rent").unwrap();
	tester.get_by_text("1800").unwrap();
	tester.get_by_text("Food").unwrap();
}

#[test]
fn a_nested_list_keeps_its_blocks_together() {
	let ui = AgentUi::new();
	let tester = mount(&ui, "- outer\n\n  - inner\n\n  After the list.\n");
	tester.get_by_text("outer").unwrap();
	tester.get_by_text("inner").unwrap();
	tester.get_by_text("After the list.").unwrap();
}

#[test]
fn a_quote_draws_a_rule_beside_its_content() {
	let ui = AgentUi::new();
	let tester = mount(&ui, "> Quoted words\n");
	tester.get_by_text("Quoted words").unwrap();
}

#[test]
fn an_empty_document_draws_nothing() {
	let ui = AgentUi::new();
	let rendered = ui.render("");
	assert_eq!(rendered.diagnostics, [] as [Diagnostic; 0]);
	assert!(!rendered.partial);
}

#[test]
fn a_document_that_never_parses_reports_the_problem() {
	let ui = AgentUi::new();
	let diagnostics = ui.check("<Card title=\"x");
	assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
	assert_ne!(diagnostics[0].message, "");

	let tester = mount(&ui, "<Card title=\"x");
	assert!(tester.surface().root().is_some());
}

#[test]
fn a_document_that_never_parses_shows_nothing_when_errors_are_off() {
	let ui = AgentUi::new().show_errors(false);
	let rendered = ui.render("<Card title=\"x");
	assert_eq!(rendered.diagnostics.len(), 1);
	// The element is the hidden placeholder rather than a note.
	let tester =
		WidgetTester::new(move || ui.render("<Card title=\"x").element, 390.0, 200.0).unwrap();
	assert!(!tester.tree().contains("⚠"));
}

#[test]
fn the_number_of_diagnostics_is_capped() {
	let ui = AgentUi::new();
	use std::fmt::Write as _;

	let mut source = String::new();
	for index in 0..105 {
		write!(source, "<Missing{index} />\n\n").unwrap();
	}
	let diagnostics = ui.check(&source);
	assert_eq!(diagnostics.len(), 100);
}

#[test]
fn an_html_tag_is_not_a_component() {
	let ui = AgentUi::new();
	let message = one(&ui, "<div style=\"position: fixed\">Pay now</div>\n");
	assert!(message.contains("there is no component <div>"), "{message}");
}

#[test]
fn fragments_are_just_their_children() {
	let ui = AgentUi::new();
	let source = "A <>fragment of text</> inside a paragraph.\n";
	let rendered = ui.render(source);
	assert!(
		rendered.diagnostics.is_empty(),
		"{:?}",
		rendered.diagnostics
	);
	let tester = mount(&ui, source);
	tester.get_by_text("A ").unwrap();
	tester.get_by_text("fragment of text").unwrap();
	tester.get_by_text(" inside a paragraph.").unwrap();
}

#[test]
fn components_can_sit_in_a_paragraph() {
	let ui = AgentUi::new();
	let tester = mount(&ui, "Before <Text>middle</Text> after\n");
	tester.get_by_text("Before ").unwrap();
	tester.get_by_text("middle").unwrap();
	tester.get_by_text(" after").unwrap();
}

#[test]
fn an_image_without_a_description_says_so() {
	let ui = AgentUi::new();
	let tester = mount(&ui, "![](https://example.com/x.png)\n");
	tester.get_by_text("[image]").unwrap();
}

#[test]
fn a_long_formula_value_is_cut_short() {
	let ui = AgentUi::new().data("long", "x".repeat(5000));
	let rendered = ui.render("{long}");
	assert!(
		rendered.diagnostics.is_empty(),
		"{:?}",
		rendered.diagnostics
	);
	let ui_for_view = ui.clone();
	let tester =
		WidgetTester::new(move || ui_for_view.render("{long}").element, 390.0, 2000.0).unwrap();
	assert!(tester.tree().contains('…'));
}

#[test]
fn a_broken_formula_is_marked_where_it_sits() {
	let ui = AgentUi::new();
	let message = one(&ui, "Rent is {1 +}\n");
	assert!(message.contains("`{1 +}`"), "{message}");
	let tester = mount(&ui, "Rent is {1 +}\n");
	assert!(tester.tree().contains("⚠"), "{}", tester.tree());

	let quiet = AgentUi::new().show_errors(false);
	let tester = mount(&quiet, "Rent is {1 +}\n");
	assert!(tester.tree().contains('—'), "{}", tester.tree());
}

#[test]
fn declarations_take_text_names_and_formula_or_literal_values() {
	let ui = AgentUi::new();
	let rendered = ui.render(
		"<Var name=\"flat\" value />\n<Var name=\"empty\" />\n<Var name=\"text\" value=\"hello\" \
		 />\n<Let name=\"twice\" value={2 * 2} />\n\n{flat} {text} {twice}\n",
	);
	assert!(
		rendered.diagnostics.is_empty(),
		"{:?}",
		rendered.diagnostics
	);
	assert_eq!(ui.value("flat"), Some(Value::Bool(true)));
	assert_eq!(ui.value("empty"), Some(Value::Null));
	assert_eq!(ui.value("text"), Some(Value::from("hello")));
	assert_eq!(ui.value("twice"), Some(Value::Number(4.0)));
}

#[test]
fn declarations_explain_what_they_cannot_take() {
	let ui = AgentUi::new();

	let found = messages(&ui, "<Var {...props} />\n");
	assert!(
		found.iter().any(|message| message.contains("spreads")),
		"{found:?}"
	);

	let found = messages(&ui, "<Var name={1} value={1} />\n");
	assert!(
		found
			.iter()
			.any(|message| message.contains("needs its name as text")),
		"{found:?}"
	);

	let message = one(&ui, "<Var name=\"x\" value={1 +} />\n");
	assert!(message.contains("`value`"), "{message}");

	let message = one(&ui, "<Var name=\"x\" extra=\"y\" />\n");
	assert!(
		message.contains("takes only `name` and `value`"),
		"{message}"
	);

	let message = one(&ui, "<Var value={1} />\n");
	assert!(message.contains("needs a `name`"), "{message}");

	let message = one(&ui, "<Var name=\"has space\" value={1} />\n");
	assert!(message.contains("cannot be a variable name"), "{message}");

	let message = one(&ui, "<Var name=\"true\" value={1} />\n");
	assert!(message.contains("cannot be a variable name"), "{message}");

	let message = one(&ui, "<Let name=\"x\" value={1 +} />\n");
	assert!(message.contains("<Let> `value`"), "{message}");
}

#[test]
fn a_value_that_starts_too_large_is_refused() {
	let ui = AgentUi::new().limits(Limits {
		max_value_bytes: 4,
		..Limits::default()
	});
	let message = one(&ui, "<Var name=\"big\" value=\"much longer\" />\n");
	assert!(message.contains("starts too large"), "{message}");
	assert_eq!(ui.value("big"), None);
}

#[test]
fn a_starting_value_that_fails_is_reported() {
	let ui = AgentUi::new();
	let message = one(&ui, "<Var name=\"x\" value={missing} />\n");
	assert!(
		message.contains("there is no variable `missing`"),
		"{message}"
	);
}

#[test]
fn a_formula_can_replace_an_input_and_an_input_can_replace_it_again() {
	let ui = AgentUi::new();
	let rendered = ui.render("<Var name=\"x\" value={1} />\n<Let name=\"x\" value={2} />\n\n{x}\n");
	assert!(
		rendered.diagnostics.is_empty(),
		"{:?}",
		rendered.diagnostics
	);
	assert_eq!(ui.value("x"), Some(Value::Number(2.0)));

	ui.render("<Var name=\"x\" value={3} />\n");
	assert_eq!(ui.value("x"), Some(Value::Number(3.0)));
}

#[test]
fn a_document_is_limited_in_variables() {
	let ui = AgentUi::new().limits(Limits {
		max_variables: 2,
		..Limits::default()
	});
	let diagnostics = ui.check(
		"<Var name=\"a\" value={1} />\n<Let name=\"b\" value={2} />\n<Var name=\"c\" value={3} \
		 />\n",
	);
	assert!(
		diagnostics
			.iter()
			.any(|diagnostic| diagnostic.message.contains("limited to 2 variables")),
		"{diagnostics:?}"
	);
}

#[test]
fn components_nest_only_as_deep_as_the_limit() {
	let ui = AgentUi::new().limits(Limits {
		max_depth: 1,
		..Limits::default()
	});
	let message = one(&ui, "<View>\n\n<View>deep</View>\n\n</View>\n");
	assert!(message.contains("nest more than 1 deep"), "{message}");
}

#[test]
fn a_component_past_the_node_limit_is_left_out() {
	let ui = AgentUi::new().limits(Limits {
		max_nodes: 1,
		..Limits::default()
	});
	let rendered = ui.render("<View>one</View>\n\n<View>two</View>\n");
	assert!(
		rendered
			.diagnostics
			.iter()
			.any(|diagnostic| diagnostic.message.contains("limited to 1 elements")),
		"{:?}",
		rendered.diagnostics
	);
}

#[test]
fn a_component_that_takes_no_content_says_so() {
	let ui = AgentUi::new();
	let message = one(&ui, "<Divider>unexpected</Divider>\n");
	assert!(message.contains("takes no content"), "{message}");
}

#[test]
fn component_attributes_explain_what_they_cannot_take() {
	let ui = AgentUi::new();

	let message = one(&ui, "<View {...props}>x</View>\n");
	assert!(message.contains("spreads"), "{message}");

	let message = one(&ui, "<View class>x</View>\n");
	assert!(message.contains("`class` needs classes"), "{message}");

	let message = one(&ui, "<View class={1 +}>x</View>\n");
	assert!(message.contains("`class`"), "{message}");

	let found = messages(&ui, "<Stat label=\"x\" value={1 +} />\n");
	assert!(
		found.iter().any(|message| message.contains("`value`")),
		"{found:?}"
	);

	let found = messages(&ui, "<Slider bind={1 + 2} min={0} max={10} />\n");
	assert!(
		found
			.iter()
			.any(|message| message.contains("names a variable")),
		"{found:?}"
	);
}

#[test]
fn a_component_whose_props_fail_shows_why() {
	let ui = AgentUi::new();
	let message = one(&ui, "<Stat label=\"x\" value={1 * \"a\"} />\n");
	assert!(message.contains("`value`"), "{message}");

	let message = one(&ui, "<Card title={1 * \"a\"}>x</Card>\n");
	assert!(message.contains("<Card>"), "{message}");
}

#[test]
fn a_dynamic_class_is_checked_once_it_has_a_value() {
	let ui = AgentUi::new();
	let rendered = ui.render(
		"<Var name=\"left\" value={-1} />\n\n<View class={left < 0 ? \"fixed\" : \
		 \"p-2\"}>x</View>\n",
	);
	assert!(
		rendered
			.diagnostics
			.iter()
			.any(|diagnostic| diagnostic.message.contains("not allowed")),
		"{:?}",
		rendered.diagnostics
	);

	let ui = AgentUi::new();
	let rendered = ui.render(
		"<Var name=\"left\" value={-1} />\n\n<View class={left < 0 ? \"bg-negative\" : \"p-2\"} \
		 >x</View>\n",
	);
	assert!(
		rendered.diagnostics.is_empty(),
		"{:?}",
		rendered.diagnostics
	);
}

#[test]
fn data_children_explain_what_they_cannot_take() {
	let ui = AgentUi::new();

	let message = one(&ui, "<Chart><Series foo=\"1\" values={[1]} /></Chart>\n");
	assert!(message.contains("has no prop `foo`"), "{message}");

	let found = messages(&ui, "<Chart><Series values={1 +} /></Chart>\n");
	assert!(
		found.iter().any(|message| message.contains("`values`")),
		"{found:?}"
	);

	let message = one(&ui, "<Chart><Series name=\"a\" values /></Chart>\n");
	assert!(message.contains("`values` must be a list"), "{message}");

	let found = messages(
		&ui,
		"<Chart><Series name=\"a\" values={missing} /></Chart>\n",
	);
	assert!(
		found
			.iter()
			.any(|message| message.contains("there is no variable `missing`")),
		"{found:?}"
	);

	// A spread among a data child's attributes is skipped, not an error.
	let rendered = ui.render("<Chart><Series {...props} name=\"a\" values={[1]} /></Chart>\n");
	assert!(
		rendered.diagnostics.is_empty(),
		"{:?}",
		rendered.diagnostics
	);
}

#[test]
fn inert_content_refuses_live_children() {
	let ui = AgentUi::new();
	let message = one(
		&ui,
		"<Pressable label=\"Open\">\n\n<View>text</View>\n\n<Button \
		 action=\"a\">Go</Button>\n\n</Pressable>\n",
	);
	assert!(message.contains("cannot hold <Button>"), "{message}");
}

#[test]
fn components_added_by_the_application_render() {
	let ui = AgentUi::new().component(
		Component::new("Echo", "Echoes its content.", |props, cx| {
			let label = props.text_or("label", "Echo");
			Ok(Text(format!("{label}: {}", cx.text())))
		})
		.prop(Prop::text("label", "What to say first."))
		.children("The text to echo."),
	);
	let tester = mount(&ui, "<Echo label=\"It says\">hello there</Echo>\n");
	tester.get_by_text("It says: hello there").unwrap();
	assert!(ui.reference().contains("### `<Echo>`"));
}

#[test]
fn every_widget_set_draws_markdown_the_same() {
	for widgets in [Widgets::All, Widgets::WithoutControls, Widgets::Basic] {
		let ui = AgentUi::new().widgets(widgets);
		let rendered = ui.render("# Title\n\nSome **text** and `code`.\n\n- one\n- two\n");
		assert!(
			rendered.diagnostics.is_empty(),
			"{widgets:?}: {:?}",
			rendered.diagnostics
		);
	}
}

#[test]
fn modern_component_errors_carry_their_line() {
	let ui = AgentUi::new();
	let diagnostics = ui.check("Fine.\n\n<Stat label=\"x\" value={1 * \"a\"} />\n");
	assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
	assert_eq!(diagnostics[0].line, 3);
	assert_eq!(
		diagnostics[0].to_string(),
		format!("3:{}: {}", diagnostics[0].column, diagnostics[0].message)
	);

	let at_zero = Diagnostic {
		line: 0,
		column: 0,
		message: "somewhere".to_owned(),
	};
	assert_eq!(at_zero.to_string(), "somewhere");
}
