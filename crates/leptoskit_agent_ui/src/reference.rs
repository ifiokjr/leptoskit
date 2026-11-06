//! The language reference an agent reads before writing documents.

use std::fmt::Write;

use crate::Inner;
use crate::expr::FUNCTIONS;
use crate::value::Value;

/// A worked example that uses most of the language.
const EXAMPLE: &str = r#"Here is where your month stands. Change the rent to see what is left.

<Var name="rent" value={1800} />
<Var name="groceries" value={600} />
<Let name="spent" value={rent + groceries} />
<Let name="left" value={4200 - spent} />

<Row equal>
  <Stat label="Spent" value={spent} format="money" />
  <Stat label="Left" value={left} format="money" delta={left - 1500} />
</Row>

<Slider bind="rent" min={1000} max={2500} step={50} label="Rent" format="money" />

<Chart type="donut" labels={["Rent", "Groceries", "Left"]} values={[rent, groceries, max(left, 0)]} format="money" />

<Show when={left < 0}>

<Callout kind="warning">You would overspend by **{money(-left)}**.</Callout>

</Show>

<Button action="apply" value={rent}>Use this rent</Button>"#;

fn plural(count: usize, noun: &str) -> String {
	if count == 1 {
		format!("{count} {noun}")
	} else {
		format!("{count} {noun}s")
	}
}

fn fields(fields: &std::collections::BTreeMap<String, Value>) -> String {
	fields
		.keys()
		.map(|key| format!("`{key}`"))
		.collect::<Vec<_>>()
		.join(", ")
}

fn describe_value(value: &Value) -> String {
	match value {
		Value::List(items) => {
			match items.first() {
				Some(Value::Record(first)) => {
					format!(
						"a list of {} with fields {}",
						plural(items.len(), "record"),
						fields(first)
					)
				}
				Some(first) => format!("a list of {}", plural(items.len(), first.kind())),
				None => "an empty list".to_owned(),
			}
		}
		Value::Record(record) => format!("a record with fields {}", fields(record)),
		other => format!("a {}", other.kind()),
	}
}

/// Writes the reference for every component `ui` knows.
pub(crate) fn write(ui: &Inner) -> String {
	let mut out = String::new();
	let limits = ui.limits;

	out.push_str(
		"# Writing interfaces\n\nYou can answer with an interface the user can read and use, not \
		 only with text. Write Markdown, and put components in it as tags, the way MDX does. The \
		 application shows your answer as native interface on the user's device as you write it. \
		 Use an interface when it helps the user see or decide something: figures, a chart, a \
		 what-if they can adjust, or a choice to make.\n\n",
	);

	out.push_str(
		"## Syntax\n\n- Markdown: headings, paragraphs, **bold**, *italic*, `code`, lists, task \
		 lists, tables, quotes, and rules. There is no HTML.\n- A component is a tag that starts \
		 with a capital letter. Close it with `/>` when it has no content: `<Stat label=\"Spent\" \
		 value={412} />`. Put a blank line after the opening tag and before the closing tag when \
		 the content is Markdown.\n- Attributes are text in quotes, `title=\"May\"`, or a formula \
		 in braces, `value={rent * 12}`. An attribute written alone, `equal`, is `true`.\n- \
		 `{formula}` in text shows its value: `Rent is {money(rent)} a month.`\n- Links show as \
		 text and cannot be opened; to offer the user a next step, use a `<Button>`.\n",
	);
	if ui.images.is_some() {
		out.push_str("- Images show only from addresses this application allows.\n");
	} else {
		out.push_str("- Images are not shown; describe what you would have shown.\n");
	}
	out.push('\n');

	out.push_str(
		"## Variables\n\n- `<Var name=\"rent\" value={1800} />` declares an input: a value the \
		 user changes with a `<Slider>`, `<Stepper>`, `<NumberField>`, `<TextField>`, `<Choice>`, \
		 `<Toggle>`, or a `<Button set=\"…\">`. Sending the document again keeps what the user \
		 set, unless you change the starting value.\n- `<Let name=\"left\" value={income - rent} \
		 />` declares a formula, recomputed whenever what it reads changes.\n- Text, figures, \
		 charts, and `<Show>` that read a variable update as soon as it changes, without a new \
		 message from you.\n- Variables are declared before anything shows, so declare them \
		 anywhere; by convention, near the top.\n",
	);

	let data = ui.store.data_summary();
	if data.is_empty() {
		out.push_str("- The application shares no data right now.\n");
	} else {
		out.push_str("- The application shares this data, which you can read and cannot change:\n");
		for (name, value) in data {
			let _ = writeln!(out, "  - `{name}`: {}", describe_value(&value));
		}
	}
	out.push('\n');

	out.push_str(
		"## Formulas\n\nFormulas are small expressions, not code. They have numbers (`1800`, \
		 `0.05`), text (`\"May\"`), `true`, `false`, `null`, lists (`[1, 2, 3]`), arithmetic (`+ \
		 - * / %`), comparisons (`== != < <= > >=`), `&&`, `||`, `!`, and `condition ? a : b`. \
		 Read a record's field with `budget.rent`, and a list's item with `months[0]`. Arithmetic \
		 on lists works item by item: `[100, 200] * 12` is `[1200, 2400]`, and \
		 `cumsum(repeat(saving, 12)) + balance` is a year of balances. `+` joins text. Dividing \
		 by zero gives `null`, which shows as nothing.\n\n| Function | What it does |\n| --- | \
		 --- |\n",
	);
	for function in FUNCTIONS {
		let _ = writeln!(out, "| `{}` | {} |", function.signature, function.summary);
	}
	out.push('\n');

	out.push_str(
		"## Actions\n\nA `<Button action=\"…\">` sends the application an action: its name, its \
		 `value` evaluated when pressed, its label, and every variable's value. The application \
		 decides what happens next, such as sending you the user's choice as their next message \
		 or asking them to confirm. Nothing happens on its own, so say what a button does in its \
		 label.\n\n",
	);

	out.push_str("## Components\n\n");
	for component in ui.components.values() {
		let _ = writeln!(out, "### `<{}>`\n\n{}\n", component.name, component.summary);
		for prop in &component.props {
			let _ = writeln!(out, "- {}", prop.describe());
		}
		if let Some(children) = &component.children {
			let _ = writeln!(out, "- Content: {children}");
		}
		if component.props.is_empty() && component.children.is_none() {
			out.push_str("- No props.\n");
		}
		if let Some(example) = &component.example {
			let _ = writeln!(out, "\n```mdx\n{example}\n```");
		}
		out.push('\n');
	}

	out.push_str(
		"## Styling\n\nThe built-in components already match the application, so style only what \
		 they do not cover. Every component takes a `class` of Tailwind utilities: layout \
		 (`flex-row`, `items-center`, `gap-3`, `grid grid-cols-2`), spacing (`p-4`, `mt-2`), size \
		 (`w-full`, `min-h-24`), type (`text-sm`, `font-semibold`, `text-center`), and shape \
		 (`rounded-xl`, `border`). With `<View>`, `<Text>`, and `<Pressable>` you can build \
		 layouts the other components do not have.\n\n- Colours are roles, never values: \
		 `bg-surface`, `text-muted`, `border-line`, `bg-accent/20` for 20% opacity. Each role \
		 takes the application's own colour and follows light and dark appearance, so do not \
		 write hex colours or palette names such as `bg-blue-500`.\n- A `class` can be a formula: \
		 `class={left < 0 ? \"text-negative\" : \"text-positive\"}`.\n- Positioning (`absolute`, \
		 `fixed`, `z-…`, offsets, negative margins) and scrolling (`overflow-auto`, \
		 `overflow-scroll`) are refused, so your content stays in its own box.\n\n| Role | For \
		 |\n| --- | --- |\n",
	);
	for (role, token) in ui.theme.roles() {
		let _ = writeln!(out, "| `{role}` | {} |", token.summary);
	}
	out.push('\n');

	let _ = write!(
		out,
		"## Rules\n\n- Keep it small: one or two ideas per answer. A document is limited to {} \
		 bytes, {} elements, and {} variables, and a chart to {} points.\n- Label everything: a \
		 chart's title or the sentence before it says what it shows, and every number says what \
		 it is.\n- Choose the chart by its job: `bar` to compare categories, `line` or `area` for \
		 change over time, `pie` or `donut` for parts of a whole with at most 8 slices.\n- Use \
		 `format=\"money\"` or `money()` for amounts.\n- Pick inputs by their job: `<Slider>` to \
		 explore a range, `<Stepper>` for small counts or exact steps, `<Choice>` for one of a \
		 few options, `<Toggle>` for on and off, and fields for typed values.\n- When something \
		 is wrong in a document, the application may send you the problems, each with its line \
		 and column. Fix them and send the whole document again.\n\n",
		limits.max_source_bytes, limits.max_nodes, limits.max_variables, limits.max_chart_points,
	);

	let _ = write!(out, "## Example\n\n```mdx\n{EXAMPLE}\n```\n");
	out
}
