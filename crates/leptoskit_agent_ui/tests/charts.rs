//! Charts: every kind, their axes, and what they refuse to draw.

use leptoskit_agent_ui::AgentUi;
use leptoskit_agent_ui::Limits;
use leptoskit_agent_ui::Widgets;
use leptoskit_testing::Queries;
use leptoskit_testing::WidgetTester;

fn mount(ui: &AgentUi, source: &'static str) -> WidgetTester {
	let ui = ui.clone();
	WidgetTester::new(move || ui.render(source).element, 390.0, 3000.0).unwrap()
}

fn messages(ui: &AgentUi, source: &str) -> Vec<String> {
	ui.check(source)
		.into_iter()
		.map(|diagnostic| diagnostic.message)
		.collect()
}

#[test]
fn bars_label_their_values_and_axis() {
	let ui = AgentUi::new();
	let source = "<Chart title=\"Spending\" labels={[\"Rent\", \"Food\", \"Fun\"]} values={[1800, \
	              600, 200]} format=\"money\" />\n";
	assert_eq!(messages(&ui, source), [] as [String; 0]);
	let tester = mount(&ui, source);
	tester.get_by_text("Spending").unwrap();
	tester.get_by_text("Rent").unwrap();
	assert_eq!(tester.get_all_by_text("$1.8K").unwrap().len(), 2);
}

#[test]
fn bars_with_negative_values_draw_two_sides() {
	let ui = AgentUi::new();
	let source = "<Chart type=\"bar\" labels={[\"Jan\", \"Feb\"]}>\n\n<Series name=\"In\" \
	              values={[4200, 4300]} />\n\n<Series name=\"Out\" values={[-3900, -4500]} \
	              />\n\n</Chart>\n";
	let rendered = ui.render(source);
	assert!(
		rendered.diagnostics.is_empty(),
		"{:?}",
		rendered.diagnostics
	);
	let tester = mount(&ui, source);
	tester.get_by_text("In").unwrap();
	tester.get_by_text("Out").unwrap();
	// Several series have no value tips, only a legend.
	tester.get_by_text("Jan").unwrap();
}

#[test]
fn many_bars_thin_their_labels() {
	let ui = AgentUi::new();
	let source = "<Chart labels={range(30)} values={range(30)} height={400} />\n";
	let rendered = ui.render(source);
	assert!(
		rendered.diagnostics.is_empty(),
		"{:?}",
		rendered.diagnostics
	);
	let tester = mount(&ui, source);
	assert_eq!(tester.get_all_by_text("0").unwrap().len(), 2);
}

#[test]
fn a_tall_chart_is_capped_and_a_short_one_raised() {
	let ui = AgentUi::new();
	for height in [10, 1000] {
		let source: &'static str = Box::leak(
			format!("<Chart labels={{[\"A\"]}} values={{[1]}} height={{{height}}} />\n")
				.into_boxed_str(),
		);
		let tester = mount(&ui, source);
		let chart = tester.get_by_label("Chart. A 1").unwrap();
		assert!(chart.frame.height <= 500.0, "{height}: {:?}", chart.frame);
	}
}

#[test]
fn lines_and_areas_handle_one_point_and_both_signs() {
	let ui = AgentUi::new();
	for (kind, values, labels) in [
		("line", "[5]", "[\"A\"]"),
		("area", "[5]", "[\"A\"]"),
		("line", "[100, -100, 50]", "[\"A\", \"B\", \"C\"]"),
		("area", "[100, -100, 50]", "[\"A\", \"B\", \"C\"]"),
	] {
		let source: &'static str = Box::leak(
			format!(
				"<Chart type=\"{kind}\" labels={{{labels}}} values={{{values}}} format=\"money\" \
				 />\n"
			)
			.into_boxed_str(),
		);
		let rendered = ui.render(source);
		assert!(
			rendered.diagnostics.is_empty(),
			"{kind} {values}: {:?}",
			rendered.diagnostics
		);
		let tester = mount(&ui, source);
		tester.get_by_text("A").unwrap();
	}
}

#[test]
fn a_line_without_labels_still_draws() {
	let ui = AgentUi::new();
	let source = "<Chart type=\"line\" values={[1, 2, 3]} />\n";
	assert_eq!(messages(&ui, source), [] as [String; 0]);
	let tester = mount(&ui, source);
	let chart = tester.get_by_label("Chart. 1, 2, 3").unwrap();
	assert!(chart.frame.height > 0.0);
}

#[test]
fn pies_and_donuts_split_a_whole() {
	let ui = AgentUi::new();
	for kind in ["pie", "donut"] {
		let source: &'static str = Box::leak(
			format!(
				"<Chart type=\"{kind}\" labels={{[\"Rent\", \"Food\"]}} values={{[600, 400]}} \
				 format=\"money\" />\n"
			)
			.into_boxed_str(),
		);
		let rendered = ui.render(source);
		assert!(
			rendered.diagnostics.is_empty(),
			"{kind}: {:?}",
			rendered.diagnostics
		);
		let tester = mount(&ui, source);
		tester.get_by_text("60%").unwrap();
		tester.get_by_text("40%").unwrap();
	}

	// One slice is a whole circle, drawn as two halves.
	let source = "<Chart type=\"donut\" values={[10]} />\n";
	let rendered = ui.render(source);
	assert!(
		rendered.diagnostics.is_empty(),
		"{:?}",
		rendered.diagnostics
	);
	let tester = mount(&ui, source);
	tester.get_by_text("Slice 1").unwrap();
	tester.get_by_text("100%").unwrap();
}

#[test]
fn pies_without_vectors_list_their_parts() {
	let ui = AgentUi::new().widgets(Widgets::Basic);
	let source = "<Chart type=\"pie\" labels={[\"Rent\", \"Food\"]} values={[600, 400]} \
	              format=\"money\" />\n";
	let rendered = ui.render(source);
	assert!(
		rendered.diagnostics.is_empty(),
		"{:?}",
		rendered.diagnostics
	);
	let tester = mount(&ui, source);
	tester.get_by_text("$600").unwrap();
	tester.get_by_text("60%").unwrap();
}

#[test]
fn many_points_are_summarized_short() {
	let ui = AgentUi::new();
	let source = "<Chart type=\"line\" values={range(30)} />\n";
	let tester = mount(&ui, source);
	let label = tester
		.get_by_label(
			"Chart. 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, \
			 22, 23, …",
		)
		.unwrap();
	assert!(label.frame.height > 0.0);
}

#[test]
fn a_chart_explains_every_thing_it_cannot_draw() {
	let ui = AgentUi::new();
	let cases = [
		("<Chart />", "needs `values`"),
		(
			"<Chart>\n\n<Series name=\"a\" values={[1]} />\n\n<Series name=\"b\" values={[1, 2]} \
			 />\n\n</Chart>",
			"same number of values",
		),
		(
			"<Chart>\n\n<Series name=\"a\" values={[1]} />\n\n<Series name=\"b\" values={[1]} \
			 />\n\n<Series name=\"c\" values={[1]} />\n\n<Series name=\"d\" values={[1]} \
			 />\n\n<Series name=\"e\" values={[1]} />\n\n<Series name=\"f\" values={[1]} \
			 />\n\n<Series name=\"g\" values={[1]} />\n\n<Series name=\"h\" values={[1]} \
			 />\n\n<Series name=\"i\" values={[1]} />\n\n</Chart>",
			"at most 8 series",
		),
		("<Chart values={[]} />", "has no values"),
		(
			"<Chart values={[pow(10, 308) * 10]} />",
			"must be finite numbers",
		),
		("<Chart type=\"radar\" values={[1]} />", "`type`"),
		(
			"<Chart type=\"pie\" values={[1, 2, 3, 4, 5, 6, 7, 8, 9]} />",
			"at most 8 slices",
		),
		("<Chart type=\"pie\" values={[0, 0]} />", "total above zero"),
		(
			"<Chart type=\"pie\" values={[1, -2]} />",
			"cannot show negative values",
		),
	];
	for (source, expected) in cases {
		let messages = messages(&ui, source);
		assert!(
			messages.iter().any(|message| message.contains(expected)),
			"{source}: {messages:?}"
		);
	}
}

#[test]
fn axis_values_compact_billions() {
	let ui = AgentUi::new();
	let source = "<Chart type=\"bar\" values={[2000000000, 1000000000]} />\n";
	let rendered = ui.render(source);
	assert_eq!(
		rendered.diagnostics,
		[] as [leptoskit_agent_ui::Diagnostic; 0]
	);
	let tester = mount(&ui, source);
	assert!(tester.tree().contains("2B"), "{}", tester.tree());
}

#[test]
fn an_all_zero_chart_still_draws() {
	let ui = AgentUi::new();
	let source = "<Chart type=\"line\" values={[0, 0]} />\n";
	let rendered = ui.render(source);
	assert!(
		rendered.diagnostics.is_empty(),
		"{:?}",
		rendered.diagnostics
	);
}

#[test]
fn a_zero_slice_leaves_a_gap() {
	let ui = AgentUi::new();
	let source = "<Chart type=\"pie\" values={[1, 0]} labels={[\"One\", \"None\"]} />\n";
	let rendered = ui.render(source);
	assert!(
		rendered.diagnostics.is_empty(),
		"{:?}",
		rendered.diagnostics
	);
	let tester = mount(&ui, source);
	tester.get_by_text("One").unwrap();
	tester.get_by_text("None").unwrap();

	let plain = AgentUi::new().widgets(Widgets::Basic);
	let source = "<Chart type=\"pie\" values={[1, 0]} />\n";
	let tester = mount(&plain, source);
	tester.get_by_text("Slice 1").unwrap();
}

#[test]
fn a_chart_stops_at_the_point_limit() {
	let ui = AgentUi::new().limits(Limits {
		max_chart_points: 5,
		..Limits::default()
	});
	let messages = messages(&ui, "<Chart values={range(6)} />\n");
	assert!(
		messages
			.iter()
			.any(|message| message.contains("at most 5 points")),
		"{messages:?}"
	);
}

#[test]
fn a_series_outside_a_chart_says_where_it_belongs() {
	let ui = AgentUi::new();
	let messages = messages(&ui, "<Series name=\"a\" values={[1]} />\n");
	assert!(
		messages
			.iter()
			.any(|message| message.contains("belongs inside a <Chart>")),
		"{messages:?}"
	);
}

#[test]
fn charts_format_percent_and_plain_axes() {
	let ui = AgentUi::new();
	for format in ["percent", "number"] {
		let source: &'static str = Box::leak(
			format!(
				"<Chart type=\"line\" labels={{[\"A\", \"B\"]}} values={{[0.25, 0.5]}} \
				 format=\"{format}\" />\n"
			)
			.into_boxed_str(),
		);
		let rendered = ui.render(source);
		assert!(
			rendered.diagnostics.is_empty(),
			"{format}: {:?}",
			rendered.diagnostics
		);
	}
}
