//! The components every document can use.
//!
//! Colours are theme roles, so every component follows the application's
//! theme and dark appearance. Controls are native where the host draws them
//! ([`Widgets::All`](crate::Widgets::All)) and built from buttons where it
//! does not, so a document never stops a host with an unsupported widget.

use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_native::Element;
use leptoskit_native::RadioGroup;
use leptoskit_native::SliderState;
use leptoskit_native::components::ActivityIndicator;
use leptoskit_native::components::Button;
use leptoskit_native::components::Checkbox;
use leptoskit_native::components::Pressable;
use leptoskit_native::components::Radio;
use leptoskit_native::components::Slider;
use leptoskit_native::components::Switch;
use leptoskit_native::components::Text;
use leptoskit_native::components::TextInput;
use leptoskit_native::components::View;
use leptoskit_yoga::Dimension;

use crate::Theme;
use crate::chart;
use crate::expr::display;
use crate::expr::grouped;
use crate::expr::money;
use crate::registry::Component;
use crate::registry::Context;
use crate::registry::Input;
use crate::registry::Prop;
use crate::registry::Props;
use crate::value::Value;
use crate::value::parse_number;

/// The most rows a `<DataTable>` shows.
const MAX_TABLE_ROWS: usize = 200;
/// The most tap targets on a slider built from buttons.
const MAX_TRACK_STOPS: usize = 11;

pub(crate) fn all() -> Vec<Component> {
	vec![
		view(),
		text(),
		stack(),
		row(),
		card(),
		callout(),
		divider(),
		show(),
		var(),
		let_(),
		stat(),
		progress(),
		spinner(),
		chart::chart().inert(),
		chart::series().inert(),
		data_table(),
		button(),
		pressable(),
		slider(),
		stepper(),
		number_field(),
		text_field(),
		choice(),
		toggle(),
	]
}

// --- Formatting -------------------------------------------------------------

/// A number as a document asked to show it: `money`, `percent` (of a
/// fraction), or a plain `number`.
pub(crate) fn shown(number: f64, format: &str) -> String {
	match format {
		"money" => money(number, "$"),
		"percent" => format!("{}%", grouped(number * 100.0, 0)),
		_ => {
			let places = if (number * 100.0).round() % 100.0 == 0.0 {
				0
			} else {
				2
			};
			grouped(number, places)
		}
	}
}

/// A value in a component, formatted when it is a number.
fn shown_value(value: &Value, format: &str) -> String {
	match value {
		Value::Number(number) => shown(*number, format),
		other => display(other),
	}
}

fn format_prop() -> Prop {
	Prop::text(
		"format",
		"How numbers show: `number`, `money`, or `percent` (of a fraction).",
	)
	.default("number")
}

/// The colour role for a tone a document names.
fn tone(tone: &str) -> &'static str {
	match tone {
		"positive" | "tip" => "positive",
		"negative" | "danger" => "negative",
		"warning" => "warning",
		_ => "accent",
	}
}

fn gap(props: &Props, default: f64) -> Result<&'static str, String> {
	Ok(match props.number_or("gap", default)?.round() as i64 {
		i64::MIN..=0 => "gap-0",
		1 => "gap-1",
		2 => "gap-2",
		3 => "gap-3",
		4 => "gap-4",
		5 => "gap-5",
		6 | 7 => "gap-6",
		_ => "gap-8",
	})
}

/// A quiet line of text: a label, caption, or note.
fn label(theme: &Theme, text: String) -> Element {
	Text(text)
		.class("text-sm")
		.class(theme.class("text", "muted"))
}

/// A secondary control's background: steppers, fields, unselected options.
fn quiet(theme: &Theme, element: Element) -> Element {
	element.class(theme.class("bg", "surface"))
}

/// A primary or selected control: accent background, on-accent text.
fn loud(theme: &Theme, element: Element) -> Element {
	element
		.class(theme.class("bg", "accent"))
		.class(theme.class("text", "on-accent"))
}

fn round_step(value: f64) -> f64 {
	(value * 1_000_000.0).round() / 1_000_000.0
}

fn optional_number(props: &Props, name: &str) -> Result<Option<f64>, String> {
	if props.has(name) {
		props.number(name).map(Some)
	} else {
		Ok(None)
	}
}

// --- Building blocks --------------------------------------------------------

fn view() -> Component {
	Component::new(
		"View",
		"A box for layouts the other components do not cover. Style it with `class`: flex \
		 direction, gaps, padding, sizes, rounded corners, borders, and colour roles. It lays its \
		 content out in a column unless `class` says `flex-row`.",
		|_props, cx| Ok(View().class("min-w-0").children(cx.take_children())),
	)
	.children("Anything: text, figures, inputs, and other views.")
	.example(
		"<View class=\"flex-row items-center gap-3 rounded-xl p-4 bg-surface\">\n  <Text \
		 class=\"flex-1 font-semibold\">Rent</Text>\n  <Text \
		 class=\"text-muted\">{money(rent)}</Text>\n</View>",
	)
	.inert()
}

fn text() -> Component {
	Component::new(
		"Text",
		"Text styled with `class`, such as `text-xs text-muted` or `text-3xl font-bold \
		 text-positive`. Formulas inside it follow their variables.",
		|_props, cx| Ok(View().class("min-w-0").children(cx.take_children())),
	)
	.children("The text, with Markdown and `{formulas}`.")
	.example("<Text class=\"text-3xl font-bold\">{money(left)}</Text>")
	.inert()
}

fn stack() -> Component {
	Component::new("Stack", "Content in a column.", |props, cx| {
		Ok(View()
			.class("flex-col min-w-0")
			.class(gap(props, 3.0)?)
			.children(cx.take_children()))
	})
	.prop(Prop::number("gap", "Space between items, from 0 to 8.").default("3"))
	.children("The items, top to bottom.")
	.inert()
}

fn row() -> Component {
	Component::new(
		"Row",
		"Content side by side, wrapping onto more lines on narrow screens.",
		|props, cx| {
			let wrap = props.bool_or("wrap", true);
			let equal = props.bool_or("equal", false);
			let align = match props.text_or("align", "center").as_str() {
				"start" => "items-start",
				"end" => "items-end",
				"stretch" => "items-stretch",
				_ => "items-center",
			};
			let justify = match props.text_or("justify", "start").as_str() {
				"center" => "justify-center",
				"end" => "justify-end",
				"between" => "justify-between",
				_ => "justify-start",
			};
			let child_class = if wrap {
				"grow shrink basis-40 min-w-0"
			} else {
				"flex-1 min-w-0"
			};
			let children = cx.take_children().into_iter().map(|child| {
				if equal {
					child.class(child_class)
				} else {
					child
				}
			});

			Ok(View()
				.class("flex-row min-w-0")
				.class(if wrap { "flex-wrap" } else { "flex-nowrap" })
				.class(align)
				.class(justify)
				.class(gap(props, 3.0)?)
				.children(children))
		},
	)
	.prop(Prop::number("gap", "Space between items, from 0 to 8.").default("3"))
	.prop(
		Prop::bool(
			"equal",
			"Gives every item the same width, as for a row of stats.",
		)
		.default("false"),
	)
	.prop(Prop::bool("wrap", "Wraps items onto more lines when they do not fit.").default("true"))
	.prop(
		Prop::text(
			"align",
			"Cross-axis alignment: `start`, `center`, `end`, or `stretch`.",
		)
		.default("center"),
	)
	.prop(
		Prop::text(
			"justify",
			"Spacing along the row: `start`, `center`, `end`, or `between`.",
		)
		.default("start"),
	)
	.children("The items, left to right.")
	.inert()
}

fn card() -> Component {
	Component::new(
		"Card",
		"A panel that groups related content.",
		|props, cx| {
			let theme = cx.theme().clone();
			let mut card = View().class("flex-col gap-2 rounded-xl p-4 min-w-0");
			card = quiet(&theme, card);

			if props.has("title") {
				card = card.child(Text(props.text("title")?).class("text-base font-semibold"));
			}
			if props.has("subtitle") {
				card = card.child(label(&theme, props.text("subtitle")?));
			}

			Ok(card.children(cx.take_children()))
		},
	)
	.prop(Prop::text("title", "A heading for the card."))
	.prop(Prop::text("subtitle", "A quieter line under the title."))
	.children("The card's content.")
	.example("<Card title=\"This month\">\n\nYou are on track.\n\n</Card>")
	.inert()
}

fn callout() -> Component {
	Component::new(
		"Callout",
		"An aside that stands out from the text around it.",
		|props, cx| {
			let kind = props.text_or("kind", "note");
			let title = props.text_or(
				"title",
				match kind.as_str() {
					"tip" => "Tip",
					"warning" => "Warning",
					"danger" => "Important",
					_ => "Note",
				},
			);
			let theme = cx.theme().clone();

			Ok(quiet(
				&theme,
				View().class("flex-row gap-3 rounded-lg p-3 min-w-0"),
			)
			.child(
				View()
					.class("w-[3px] rounded-full")
					.class(theme.class("bg", tone(&kind))),
			)
			.child(
				View()
					.class("flex-col gap-1 flex-1 min-w-0")
					.child(Text(title).class("font-semibold"))
					.children(cx.take_children()),
			))
		},
	)
	.prop(Prop::text("kind", "`note`, `tip`, `warning`, or `danger`.").default("note"))
	.prop(Prop::text("title", "Replaces the title the kind gives it."))
	.children("What the callout says.")
	.inert()
}

fn divider() -> Component {
	Component::new(
		"Divider",
		"A horizontal rule between sections.",
		|_props, cx| {
			Ok(View()
				.class("h-px w-full")
				.class(cx.theme().class("bg", "line")))
		},
	)
	.inert()
}

fn show() -> Component {
	Component::new(
		"Show",
		"Content shown only while a condition holds; it follows the variables the condition reads.",
		|props, cx| {
			if props.get("when").truthy() {
				Ok(View()
					.class("flex-col gap-3 min-w-0")
					.children(cx.take_children()))
			} else {
				Ok(View().class("hidden"))
			}
		},
	)
	.prop(Prop::any("when", "The condition, such as `{left < 0}`.").required())
	.children("What to show.")
	.example("<Show when={left < 0}>\n\nYou are over budget.\n\n</Show>")
	.inert()
}

fn var() -> Component {
	Component::new(
		"Var",
		"Declares an input variable the user can change, with its starting value. Inputs and \
		 buttons write it; formulas, text, and charts that read it follow it. It shows nothing.",
		|_props, _cx| Ok(View().class("hidden")),
	)
	.prop(
		Prop::text(
			"name",
			"The variable's name: letters, digits, and underscores.",
		)
		.required(),
	)
	.prop(Prop::any("value", "The starting value.").required())
	.example("<Var name=\"rent\" value={1800} />")
	.inert()
}

fn let_() -> Component {
	Component::new(
		"Let",
		"Declares a formula variable, recomputed whenever the variables it reads change. It shows \
		 nothing.",
		|_props, _cx| Ok(View().class("hidden")),
	)
	.prop(Prop::text("name", "The variable's name.").required())
	.prop(Prop::any("value", "The formula.").required())
	.example("<Let name=\"yearly\" value={rent * 12} />")
	.inert()
}

// --- Figures ----------------------------------------------------------------

fn stat() -> Component {
	Component::new(
		"Stat",
		"A headline number with a label, and optionally its change.",
		|props, cx| {
			let theme = cx.theme().clone();
			let mut tile = quiet(
				&theme,
				View().class("flex-col gap-1 rounded-xl p-4 min-w-0"),
			)
			.child(label(&theme, props.text("label")?))
			.child(
				Text(shown_value(
					props.get("value"),
					&props.text_or("format", "number"),
				))
				.class("text-2xl font-semibold"),
			);

			if props.has("delta") {
				let delta = props.get("delta");
				let falling = match delta {
					Value::Number(number) => Some(*number < 0.0),
					other => {
						let text = display(other);
						let text = text.trim_start();
						if text.starts_with(['-', '−']) {
							Some(true)
						} else if text.starts_with('+') {
							Some(false)
						} else {
							None
						}
					}
				};
				let good_when_rising = props.text_or("good", "up") != "down";
				let mut shown = shown_value(delta, &props.text_or("format", "number"));
				if let (Some(false), Value::Number(_)) = (falling, delta) {
					shown.insert(0, '+');
				}

				let text = match falling {
					Some(true) => Text(format!("▼ {shown}")),
					Some(false) => Text(format!("▲ {shown}")),
					None => Text(shown),
				}
				.class("text-sm");
				let role = match falling {
					Some(falling) if falling == good_when_rising => "negative",
					Some(_) => "positive",
					None => "muted",
				};
				tile = tile.child(text.class(theme.class("text", role)));
			}

			if props.has("caption") {
				tile = tile.child(label(&theme, props.text("caption")?));
			}

			Ok(tile)
		},
	)
	.prop(Prop::text("label", "What the number is.").required())
	.prop(Prop::any("value", "The number, or text such as `{money(left)}`.").required())
	.prop(format_prop())
	.prop(Prop::any(
		"delta",
		"The change, such as `{-120}` or `\"+4% vs May\"`; it shows an arrow.",
	))
	.prop(Prop::text("good", "Which direction is good news: `up` or `down`.").default("up"))
	.prop(Prop::text("caption", "A quieter line under the number."))
	.example("<Stat label=\"Left to spend\" value={left} format=\"money\" delta={-120} />")
	.inert()
}

fn progress() -> Component {
	Component::new(
		"Progress",
		"A meter: how much of a total is used, such as a category's budget. It turns red past the \
		 total.",
		|props, cx| {
			let value = props.number("value")?;
			let max = props.number_or("max", 100.0)?;
			if max <= 0.0 {
				return Err("`max` must be more than 0".to_owned());
			}

			let format = props.text_or("format", "number");
			let over = value > max;
			let theme = cx.theme().clone();
			let fill = if over {
				"negative"
			} else {
				tone(&props.text_or("tone", "accent"))
			};
			let fraction = (value / max).clamp(0.0, 1.0) as f32;
			let amount = format!("{} of {}", shown(value, &format), shown(max, &format));

			let mut header = View().class("flex-row gap-2 items-center min-w-0");
			if props.has("label") {
				header = header.child(Text(props.text("label")?).class("flex-1 min-w-0 text-sm"));
			}
			header = header.child(
				Text(amount)
					.class("text-sm")
					.class(theme.class("text", if over { "negative" } else { "muted" })),
			);

			Ok(View()
				.class("flex-col gap-1.5 min-w-0")
				.aria_label(format!(
					"{}: {} of {}",
					props.text_or("label", "Progress"),
					shown(value, &format),
					shown(max, &format)
				))
				.child(header)
				.child(meter(&theme, fraction, fill, "h-2")))
		},
	)
	.prop(Prop::number("value", "How much is used.").required())
	.prop(Prop::number("max", "The total.").default("100"))
	.prop(Prop::text("label", "What is measured."))
	.prop(format_prop())
	.prop(
		Prop::text(
			"tone",
			"The fill: `accent`, `positive`, `warning`, or `negative`.",
		)
		.default("accent"),
	)
	.example("<Progress label=\"Groceries\" value={412} max={600} format=\"money\" />")
	.inert()
}

/// A rounded track filled to `fraction` with `role`.
pub(crate) fn meter(theme: &Theme, fraction: f32, role: &str, height: &str) -> Element {
	View()
		.class("w-full rounded-full overflow-hidden")
		.class(height)
		.class(theme.class("bg", "line"))
		.child(
			View()
				.class("h-full rounded-full")
				.class(theme.class("bg", role))
				.with_style(|style| style.width = Dimension::Percent(fraction.clamp(0.0, 1.0))),
		)
}

fn spinner() -> Component {
	Component::new(
		"Spinner",
		"Shows that something is still being worked out, such as while you wait for the \
		 application.",
		|props, cx| {
			let text = props.text_or("label", "Loading");
			let theme = cx.theme().clone();
			let row = View().class("flex-row gap-2 items-center min-w-0");
			if cx.widgets().controls() {
				Ok(row
					.child(ActivityIndicator(true).aria_label(text.clone()))
					.child(label(&theme, text).aria_hidden(true)))
			} else {
				Ok(row.child(label(&theme, format!("{text}…"))))
			}
		},
	)
	.prop(Prop::text("label", "What is happening.").default("Loading"))
	.inert()
}

fn data_table() -> Component {
	Component::new(
		"DataTable",
		"A table of records, such as transactions from the application's data. For a table you \
		 write out yourself, use a Markdown table.",
		|props, cx| {
			let rows = props.list("rows")?;
			let columns = match props.texts("columns")? {
				columns if !columns.is_empty() => columns,
				_ => {
					match rows.first() {
						Some(Value::Record(fields)) => fields.keys().cloned().collect(),
						_ => {
							return Err(
								"`columns` is required when `rows` holds no records".to_owned()
							);
						}
					}
				}
			};
			let headers = props.texts("headers")?;
			let money_columns = props.texts("money")?;
			let shown_rows = rows.len().min(MAX_TABLE_ROWS);
			cx.reserve((shown_rows + 1) * (columns.len() + 2))?;
			let theme = cx.theme().clone();
			let rule = theme.class("bg", "line");

			let header = View()
				.class("flex-row min-w-0")
				.children(columns.iter().enumerate().map(|(index, column)| {
					let title = headers.get(index).unwrap_or(column);
					View()
						.class("flex-1 min-w-0 px-2 py-1.5")
						.child(Text(title.clone()).class("text-sm font-semibold"))
				}));

			let mut table = View().class("flex-col min-w-0").child(header);
			for row in &rows[..shown_rows] {
				let cells = columns.iter().map(|column| {
					let value = match row {
						Value::Record(fields) => fields.get(column).cloned().unwrap_or_default(),
						_ => Value::Null,
					};
					let text = match (&value, money_columns.contains(column)) {
						(Value::Number(number), true) => money(*number, "$"),
						(value, _) => display(value),
					};
					View()
						.class("flex-1 min-w-0 px-2 py-1.5")
						.child(Text(text).class("text-sm"))
				});
				table = table
					.child(View().class("h-px w-full").class(&rule))
					.child(View().class("flex-row min-w-0").children(cells));
			}

			if rows.len() > shown_rows {
				table = table.child(label(
					&theme,
					format!("{} more rows not shown", rows.len() - shown_rows),
				));
			}

			Ok(table)
		},
	)
	.prop(Prop::list("rows", "A list of records.").required())
	.prop(Prop::list(
		"columns",
		"The fields to show, in order; every field of the first row when left out.",
	))
	.prop(Prop::list(
		"headers",
		"Column titles, in the same order; the field names when left out.",
	))
	.prop(Prop::list("money", "Fields to show as amounts."))
	.example(
		"<DataTable rows={transactions} columns={[\"date\", \"payee\", \"amount\"]} \
		 money={[\"amount\"]} />",
	)
	.inert()
}

// --- Actions ----------------------------------------------------------------

/// What pressing does: set a variable (`set` and `to`), send an action
/// (`action` and `value`), or both. `None` when it does neither, so the
/// control shows as disabled.
fn press(
	props: &Props,
	cx: &Context,
	label: &str,
) -> Result<Option<impl Fn() + Send + Sync + 'static>, String> {
	let setter = if props.has("set") {
		Some((cx.input(props, "set")?, props.get("to").clone()))
	} else {
		None
	};
	let action = if props.has("action") {
		cx.action(&props.text("action")?, props.get("value").clone(), label)
	} else {
		None
	};
	if setter.is_none() && action.is_none() {
		return Ok(None);
	}

	Ok(Some(move || {
		if let Some((input, value)) = &setter {
			input.set(value.clone());
		}
		if let Some(action) = &action {
			action();
		}
	}))
}

fn press_props(component: Component) -> Component {
	component
		.prop(Prop::text(
			"action",
			"The action to send the application, such as `apply` or `ask`.",
		))
		.prop(Prop::any(
			"value",
			"Sent with the action, evaluated when pressed.",
		))
		.prop(Prop::variable("set", "An input variable to set when pressed.").unfollowed())
		.prop(Prop::any("to", "The value `set` receives."))
}

fn button() -> Component {
	press_props(
		Component::new(
			"Button",
			"A button. It can set a variable (`set` and `to`), send an action to the application \
			 (`action` and `value`), or both. Actions reach the application, which decides what \
			 happens, such as sending you the user's choice.",
			|props, cx| {
				let label = match props.text_or("label", "") {
					label if !label.is_empty() => label,
					_ => cx.text().to_owned(),
				};
				if label.is_empty() {
					return Err("needs a label, between its tags or as `label`".to_owned());
				}

				let pressed = press(props, cx, &label)?;
				let theme = cx.theme().clone();
				let element = Button(label)
					.class("rounded-lg px-4 py-2 font-medium")
					.enabled(pressed.is_some());
				let element = if props.text_or("variant", "primary") == "secondary" {
					quiet(&theme, element)
				} else {
					loud(&theme, element)
				};

				Ok(element.on_press(move || {
					if let Some(pressed) = &pressed {
						pressed();
					}
				}))
			},
		)
		.prop(Prop::text(
			"label",
			"The label, when it is not written between the tags.",
		)),
	)
	.prop(Prop::text("variant", "`primary` or `secondary`.").default("primary"))
	.children("The label.")
	.example("<Button action=\"apply\" value={rent}>Use this rent</Button>")
}

fn pressable() -> Component {
	press_props(
		Component::new(
			"Pressable",
			"Content the user presses as a whole, such as a card for each option: text, figures, \
			 and layout inside, styled with `class`. It takes the same `action`, `value`, `set`, \
			 and `to` as a `<Button>`. It cannot hold buttons or inputs.",
			|props, cx| {
				let label = props.text("label")?;
				if label.trim().is_empty() {
					return Err("needs a `label` that says what pressing it does".to_owned());
				}
				let pressed = press(props, cx, &label)?;
				let usable = pressed.is_some();
				let on_press = move || {
					if let Some(pressed) = &pressed {
						pressed();
					}
				};

				if !cx.widgets().vectors() {
					// A host without pressables shows a button that says the
					// same thing.
					return Ok(
						quiet(cx.theme(), Button(label).class("rounded-lg px-4 py-2"))
							.enabled(usable)
							.on_press(on_press),
					);
				}

				let element = Pressable(label)
					.class("flex-col min-w-0")
					.children(cx.take_children())
					.enabled(usable)
					.on_press(on_press);
				element
					.validate()
					.map_err(|error| format!("cannot hold its content: {error}"))?;
				Ok(element)
			},
		)
		.prop(
			Prop::text(
				"label",
				"What pressing it does, for screen readers and for hosts that show a plain button \
				 instead.",
			)
			.required(),
		),
	)
	.children("What it shows: text, figures, and layout, without buttons or inputs.")
	.example(
		"<Pressable label=\"Choose the steady plan\" action=\"choose\" value=\"steady\" \
		 class=\"rounded-xl p-4 gap-1 bg-surface\">\n  <Text \
		 class=\"font-semibold\">Steady</Text>\n  <Text class=\"text-sm \
		 text-muted\">{money(saving)} a month</Text>\n</Pressable>",
	)
	.inert_content()
}

// --- Inputs -----------------------------------------------------------------

/// `value` on the slider's grid: within `min` and `max`, and on
/// `min + n * step` when it has a step.
fn snap(value: f64, min: f64, max: f64, step: Option<f64>) -> f64 {
	let value = value.clamp(min, max);
	let Some(step) = step else {
		return value;
	};
	let steps = ((value - min) / step).round();
	let last = ((max - min) / step).floor();
	min + steps.min(last) * step
}

fn slider() -> Component {
	Component::new(
		"Slider",
		"Drags a number variable between `min` and `max`, and shows its value. Use it for a \
		 what-if the user explores, such as a monthly saving.",
		|props, cx| {
			let input = cx.input(props, "bind")?;
			let min = props.number("min")?;
			let max = props.number("max")?;
			if min >= max {
				return Err("`max` must be more than `min`".to_owned());
			}
			let step = optional_number(props, "step")?.map(f64::abs);
			if step.is_some_and(|step| step <= 0.0 || step > max - min) {
				return Err("`step` must be more than 0 and at most `max - min`".to_owned());
			}
			let format = props.text_or("format", "number");
			let current = props.get("bind.value").as_number().unwrap_or(min);
			let value = snap(current, min, max, step);
			let name = props.text_or("label", "Value");
			let theme = cx.theme().clone();

			// The slider announces its own name and value, so the header is
			// only for the eye.
			let header = View()
				.class("flex-row gap-2 items-center min-w-0")
				.aria_hidden(true)
				.child(Text(name.clone()).class("flex-1 min-w-0"))
				.child(Text(shown(value, &format)).class("font-semibold"));

			let control = if cx.widgets().controls() {
				let state = SliderState::new(min, max, value, step)
					.map_err(|error| format!("cannot slide over that range: {error}"))?;
				Slider(state)
					.class("w-full")
					.aria_label(name)
					.aria_value_text(shown(value, &format))
					.on_value_change(move |value| input.set(Value::Number(round_step(value))))
			} else {
				cx.reserve(MAX_TRACK_STOPS + 4)?;
				track(&theme, &input, (min, max, step), value, &name, &format)
			};

			Ok(View()
				.class("flex-col gap-1 min-w-0")
				.child(header)
				.child(control))
		},
	)
	.prop(Prop::variable("bind", "The number variable it changes.").required())
	.prop(Prop::number("min", "The lowest value.").required())
	.prop(Prop::number("max", "The highest value.").required())
	.prop(Prop::number(
		"step",
		"The smallest change; any value between `min` and `max` when left out.",
	))
	.prop(Prop::text("label", "What it changes."))
	.prop(format_prop())
	.example(
		"<Slider bind=\"saving\" min={0} max={2000} step={50} label=\"Monthly saving\" \
		 format=\"money\" />",
	)
}

/// A slider for hosts without one: a row of stops the user taps to jump to,
/// filled up to the value, between − and + buttons for single steps.
fn track(
	theme: &Theme,
	input: &Input,
	(min, max, step): (f64, f64, Option<f64>),
	value: f64,
	name: &str,
	format: &str,
) -> Element {
	let fine = step.unwrap_or((max - min) / 100.0);
	let count = match step {
		Some(step) => (((max - min) / step).floor() as usize + 1).min(MAX_TRACK_STOPS),
		None => MAX_TRACK_STOPS,
	}
	.max(2);

	let set = |target: f64| {
		let input = input.clone();
		let target = round_step(snap(target, min, max, step));
		move || input.set(Value::Number(target))
	};

	let stops = (0..count).map(|index| {
		let target = snap(
			min + (max - min) * index as f64 / (count - 1) as f64,
			min,
			max,
			step,
		);
		let reached = target <= value + fine / 2.0;
		Button("")
			.class("flex-1 h-6 rounded-md")
			.class(theme.class("bg", if reached { "accent" } else { "line" }))
			.aria_label(format!("Set {name} to {}", shown(target, format)))
			.on_press(set(target))
	});

	let nudge = |symbol: &str, direction: f64, enabled: bool, verb: &str| {
		quiet(
			theme,
			Button(symbol).class("w-8 h-8 rounded-lg font-semibold"),
		)
		.enabled(enabled)
		.aria_label(format!("{verb} {name} by {}", shown(fine, format)))
		.on_press(set(value + direction * fine))
	};

	View()
		.class("flex-row gap-2 items-center min-w-0")
		.child(nudge("−", -1.0, value > min, "Decrease"))
		.child(
			View()
				.class("flex-1 flex-row gap-1 min-w-0")
				.children(stops),
		)
		.child(nudge("+", 1.0, value < max, "Increase"))
}

fn stepper() -> Component {
	Component::new(
		"Stepper",
		"Changes a number variable in steps with − and + buttons, for small counts or exact \
		 amounts; use a `<Slider>` to explore a range.",
		|props, cx| {
			let input = cx.input(props, "bind")?;
			let step = props.number_or("step", 1.0)?.abs();
			let min = optional_number(props, "min")?;
			let max = optional_number(props, "max")?;
			let format = props.text_or("format", "number");
			let current = props.get("bind.value").as_number().unwrap_or(0.0);
			let theme = cx.theme().clone();

			let change = |direction: f64| {
				let input = input.clone();
				move || {
					let value = input.get().as_number().unwrap_or(0.0) + direction * step;
					let value = min.map_or(value, |min| value.max(min));
					let value = max.map_or(value, |max| value.min(max));
					input.set(Value::Number(round_step(value)));
				}
			};
			let control = |symbol: &str, enabled: bool| {
				quiet(
					&theme,
					Button(symbol).class("w-10 h-10 rounded-lg text-lg font-semibold"),
				)
				.enabled(enabled)
			};

			let mut row = View().class("flex-row gap-2 items-center min-w-0");
			if props.has("label") {
				row = row.child(Text(props.text("label")?).class("flex-1 min-w-0"));
			}

			Ok(row
				.child(
					control("−", min.is_none_or(|min| current > min))
						.aria_label(format!("Decrease by {}", shown(step, &format)))
						.on_press(change(-1.0)),
				)
				.child(
					Text(shown(current, &format)).class("min-w-[88px] text-center font-semibold"),
				)
				.child(
					control("+", max.is_none_or(|max| current < max))
						.aria_label(format!("Increase by {}", shown(step, &format)))
						.on_press(change(1.0)),
				))
		},
	)
	.prop(Prop::variable("bind", "The number variable it changes.").required())
	.prop(Prop::number("step", "How much one press changes it.").default("1"))
	.prop(Prop::number("min", "The lowest value."))
	.prop(Prop::number("max", "The highest value."))
	.prop(Prop::text("label", "What it changes."))
	.prop(format_prop())
	.example("<Stepper bind=\"rent\" step={50} min={0} label=\"Rent\" format=\"money\" />")
}

fn field(props: &Props, cx: &Context, number: bool) -> Result<Element, String> {
	let input = cx.input(props, "bind")?;
	let initial = display(&input.current());
	let theme = cx.theme().clone();
	let mut field = quiet(&theme, TextInput().class("rounded-lg px-3 py-2 min-w-0"))
		.value(initial)
		.on_change(move |text| {
			if number {
				if let Some(value) = parse_number(text) {
					input.set(Value::Number(value));
				} else if text.trim().is_empty() {
					input.set(Value::Number(0.0));
				}
			} else {
				input.set(Value::Text(text.to_owned()));
			}
		});

	if props.has("placeholder") {
		field = field.placeholder(props.text("placeholder")?);
	}

	if !props.has("label") {
		return Ok(field);
	}

	let label_text = props.text("label")?;
	Ok(View()
		.class("flex-col gap-1 min-w-0")
		.child(label(&theme, label_text.clone()))
		.child(field.aria_label(label_text)))
}

fn number_field() -> Component {
	Component::new(
		"NumberField",
		"A field where the user types a number into a variable, such as an amount.",
		|props, cx| field(props, cx, true),
	)
	.prop(
		Prop::variable("bind", "The number variable it writes.")
			.required()
			.unfollowed(),
	)
	.prop(Prop::text("label", "What to type."))
	.prop(Prop::text("placeholder", "Shown while the field is empty."))
}

fn text_field() -> Component {
	Component::new(
		"TextField",
		"A field where the user types text into a variable, such as a note.",
		|props, cx| field(props, cx, false),
	)
	.prop(
		Prop::variable("bind", "The text variable it writes.")
			.required()
			.unfollowed(),
	)
	.prop(Prop::text("label", "What to type."))
	.prop(Prop::text("placeholder", "Shown while the field is empty."))
}

/// A radio group name no other choice on screen shares, so each group's
/// selection stays its own.
fn radio_group() -> RadioGroup {
	static NEXT: AtomicU64 = AtomicU64::new(0);
	let id = NEXT.fetch_add(1, Ordering::Relaxed);
	RadioGroup::new(format!("agent-choice-{id}")).expect("a short printable name")
}

fn choice() -> Component {
	Component::new(
		"Choice",
		"Picks one of a few options into a variable, as a row of buttons or, with \
		 `kind=\"radio\"`, a list of radio options.",
		|props, cx| {
			let input = cx.input(props, "bind")?;
			let options = props.list("options")?.to_vec();
			let labels = props.texts("labels")?;
			let current = props.get("bind.value").clone();
			let theme = cx.theme().clone();
			let radio = props.text_or("kind", "buttons") == "radio";
			cx.reserve(options.len() * 2)?;

			let text = |index: usize, option: &Value| {
				labels
					.get(index)
					.cloned()
					.unwrap_or_else(|| display(option))
			};

			let mut choice = View().class("flex-col gap-1 min-w-0");
			if props.has("label") {
				choice = choice.child(label(&theme, props.text("label")?));
			}

			if radio && cx.widgets().controls() {
				for (index, option) in options.iter().enumerate() {
					if options[..index].contains(option) {
						return Err(format!("lists {} twice", display(option)));
					}
				}
				let group = radio_group();
				let rows = options.into_iter().enumerate().map(|(index, option)| {
					let text = text(index, &option);
					let selected = option == current;
					let input = input.clone();
					View()
						.class("flex-row gap-2 items-center min-h-8 min-w-0")
						.child(
							Radio(group.clone(), selected)
								.aria_label(text.clone())
								.on_toggle(move |checked| {
									if checked {
										input.set(option.clone());
									}
								}),
						)
						.child(Text(text).class("flex-1 min-w-0").aria_hidden(true))
				});
				return Ok(choice.children(rows));
			}

			let buttons = options.into_iter().enumerate().map(|(index, option)| {
				let selected = option == current;
				let button = Button(text(index, &option)).class("rounded-lg px-3 py-2");
				let button = if selected {
					loud(&theme, button)
				} else {
					quiet(&theme, button)
				};
				let input = input.clone();
				button.on_press(move || input.set(option.clone()))
			});

			Ok(choice.child(
				View()
					.class(if radio {
						"flex-col gap-2 min-w-0"
					} else {
						"flex-row flex-wrap gap-2 min-w-0"
					})
					.children(buttons),
			))
		},
	)
	.prop(Prop::variable("bind", "The variable it sets.").required())
	.prop(Prop::list("options", "The values to choose between.").required())
	.prop(Prop::list(
		"labels",
		"What each option says, in the same order; the values when left out.",
	))
	.prop(Prop::text("label", "What is being chosen."))
	.prop(
		Prop::text(
			"kind",
			"`buttons`, a row for two to four short options, or `radio`, a list for longer ones.",
		)
		.default("buttons"),
	)
	.example(
		"<Choice bind=\"plan\" options={[\"steady\", \"aggressive\"]} labels={[\"Steady\", \
		 \"Aggressive\"]} />",
	)
}

fn toggle() -> Component {
	Component::new(
		"Toggle",
		"Turns a true-or-false variable on and off, as a switch or, with `kind=\"checkbox\"`, a \
		 checkbox.",
		|props, cx| {
			let input = cx.input(props, "bind")?;
			let on = props.get("bind.value").truthy();
			let text = props.text("label")?;
			let theme = cx.theme().clone();
			let checkbox = props.text_or("kind", "switch") == "checkbox";

			if !cx.widgets().controls() {
				return Ok(quiet(
					&theme,
					Button(format!("{} {text}", if on { "☑" } else { "☐" }))
						.class("rounded-lg px-3 py-2"),
				)
				.on_press(move || {
					let next = !input.get().truthy();
					input.set(Value::Bool(next));
				}));
			}

			let control = if checkbox { Checkbox(on) } else { Switch(on) }
				.aria_label(text.clone())
				.on_toggle(move |checked| input.set(Value::Bool(checked)));
			let words = Text(text).class("flex-1 min-w-0").aria_hidden(true);
			let row = View().class("flex-row gap-3 items-center min-h-8 min-w-0");
			Ok(if checkbox {
				row.child(control).child(words)
			} else {
				row.child(words).child(control)
			})
		},
	)
	.prop(Prop::variable("bind", "The variable it turns on and off.").required())
	.prop(Prop::text("label", "What it turns on.").required())
	.prop(Prop::text("kind", "`switch` or `checkbox`.").default("switch"))
	.example("<Toggle bind=\"round_up\" label=\"Round purchases up to save\" />")
}
