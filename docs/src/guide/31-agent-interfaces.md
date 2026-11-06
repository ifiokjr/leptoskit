# Interfaces from agents

An assistant inside your app often has more to say than text fits. A budgeting assistant wants to show where the month's money went, let the user try a lower rent and see what is left, and offer to apply the change. You can't ship a new build for every answer it might give, and you shouldn't run code it writes.

`leptoskit_agent_ui` lets the agent answer with an interface anyway. The agent writes Markdown with components in it, the way MDX mixes them, and your app renders that text as native interface while it runs:

```mdx
Here is where your month stands. Change the rent to see what is left.

<Var name="rent" value={1800} />
<Let name="left" value={income - rent - groceries} />

<Stat label="Left to spend" value={left} format="money" />

<Slider bind="rent" min={1000} max={2500} step={50} label="Rent" format="money" />

<Chart type="donut" labels={["Rent", "Groceries", "Left"]} values={[rent, groceries, max(left, 0)]} format="money" />

<Button action="apply" value={rent}>Use this rent</Button>
```

When the user drags the slider, the figure and the chart follow at once, without asking the agent again. The document is data, not code. It can lay out text, show figures and charts, declare variables the user changes, compute with formulas that follow those variables, and offer buttons. It can't run anything, reach your app's state beyond what you share, or do anything when a button is pressed: your app decides that.

Because the documents are text, an app already in the stores shows them without an update. The interpreter is part of your app's interface bundle, so it reaches published apps with an ordinary [code push](05-code-push.md), and after that every new document is just a message from the agent.

## Install

```toml
[dependencies]
leptoskit = { version = "0.4", features = ["agent-ui"] }
```

Or depend on `leptoskit_agent_ui` directly.

## Show an answer

Create one `AgentUi` for the conversation and keep it: it holds the variables the user changes, so their input survives the agent sending the document again. Then render the agent's answer, which can still be streaming in:

```rust,ignore
use leptoskit::agent_ui::AgentUi;
use leptoskit::prelude::*;

#[component]
pub fn Assistant(ui: AgentUi, answer: RwSignal<String>) -> Element {
	view! {
		<ScrollView class="p-4">
			{ui.view(move || answer.get())}
		</ScrollView>
	}
}

let ui = AgentUi::new()
	.data("income", 4200)
	.data("groceries", 600)
	.on_action(move |action| send_to_agent(action));
```

`ui.view` follows the signal, so the answer redraws as text arrives. A component the agent has opened and not closed yet shows with the text it has so far. Only the parts that read a variable redraw when the user changes it, without a new message from the agent.

`ui.render(text)` renders one document and also returns its `diagnostics` and whether it was `partial`.

## Teach the agent the language

`ui.reference()` returns the language reference as Markdown: the syntax, the formula functions, your shared data, and every component with its props and an example. Put it in the agent's instructions, or in the skill it loads.

The reference describes the components registered when you call it. When a code push adds a component, the agent learns about it from the same update.

## Get answers back

A button sends an `Action` to your `on_action` handler, with the button's `action` name, its `value`, its label, and the value of every variable:

```rust,ignore
let ui = AgentUi::new().on_action(move |action| {
	match action.name.as_str() {
		// Saving a budget is your app's decision, not the agent's.
		"apply" => confirm_and_save_rent(&action.value),
		// Anything else goes back to the agent as the user's reply.
		_ => send_to_agent(format!("{}: {}", action.label, action.value)),
	}
});
```

Without a handler, buttons that send actions are disabled. A button can also change a variable on its own, with `set` and `to`, for interactions that need no reply.

When a document has problems, such as a misspelled component, a formula that reads a variable that doesn't exist, or a chart with more labels than values, the part that failed shows a short note in its place and the rest renders. `ui.check(text)` lists the problems with their line and column. Send them back to the agent so it can fix the document. Turn the notes off for people who aren't the document's author with `.show_errors(false)`.

## Share your data

`.data(name, value)` gives documents a read-only variable: a number, text, a list, or a record such as a budget. Records read as `budget.rent` and lists of records feed `<DataTable rows={transactions} />` or `pluck(transactions, "amount")`. `ui.set_data(name, value)` updates it later, and everything that reads it follows. A document can't change your data or declare a variable with the same name.

## Add your own components

Register components the agent can use by name. Their description and props go into the reference:

```rust,ignore
use leptoskit::agent_ui::{AgentUi, Component, Prop};

let ui = AgentUi::new().component(
	Component::new(
		"Envelope",
		"A spending envelope: how much of a category's budget is spent.",
		|props, _cx| {
			let name = props.text("name")?;
			let spent = props.number("spent")?;
			let limit = props.number("limit")?;
			Ok(view! { <EnvelopeCard name=name spent=spent limit=limit /> })
		},
	)
	.prop(Prop::text("name", "The category.").required())
	.prop(Prop::number("spent", "Spent so far this month.").required())
	.prop(Prop::number("limit", "The month's budget.").required())
	.example(r#"<Envelope name="Groceries" spent={412} limit={600} />"#),
);
```

A component whose props read variables renders again when they change. `.without("Chart")` removes a component, built-in or yours.

## Match your app's look

A document never names a colour. The agent styles with Tailwind classes and colour roles, such as `bg-surface`, `text-muted`, and `border-accent/40`, and each role takes your app's colour in light and dark appearance:

```mdx
<View class="flex-row items-center gap-3 rounded-xl p-4 bg-surface">
  <Text class="flex-1 font-semibold">Groceries</Text>
  <Text class={left < 0 ? "text-negative" : "text-positive"}>{money(left)}</Text>
</View>
```

| Role                              | For                                                |
| --------------------------------- | -------------------------------------------------- |
| `accent`, `on-accent`             | Primary buttons, selected options, links, meters   |
| `positive`, `warning`, `negative` | Good news, something to watch, bad news            |
| `surface`, `line`                 | Behind cards and fields; dividers and chart tracks |
| `muted`                           | Labels, captions, and axis values                  |
| `series-1` to `series-8`          | Chart series, in order                             |

Each role reads the [theme variable](03-configuration.md#theme) of the same name, and in dark appearance the one ending in `-dark`, falling back to the light one. So the theme you already set in `leptoskit.toml` restyles every document, and the built-in components with it:

```toml
[theme.colors]
accent = "#0f766e"
accent-dark = "#2dd4bf"
surface = "#f4efe6"
surface-dark = "#2a2620"
```

A role your app doesn't set shows a default chosen to read well on light and dark backgrounds; the chart series are a colour-blind-safe order with separate light and dark steps. Text keeps your app's own colour. Dark appearance switches on the device without a new render.

When your theme already uses other names, point a role at them, and add roles of your own for the agent to use:

```rust,ignore
use leptoskit::agent_ui::{AgentUi, Theme};
use leptoskit::prelude::Color;

let ui = AgentUi::new().theme(
	Theme::default()
		// Read `--brand`, and `--brand-dark` in dark appearance.
		.variable("accent", "brand")
		// Documents can now write `bg-savings` and `text-savings`.
		.color(
			"savings",
			Color::rgba(15, 118, 110, 255),
			Color::rgba(45, 212, 191, 255),
			"Money set aside.",
		),
);
```

The reference lists every role, so the agent knows yours. A `class` can also be a formula, as in the example above, and your own components can use roles too, with `cx.style("bg-surface p-4")`.

## Built-in components

| Purpose         | Components                                                                                                   |
| --------------- | ------------------------------------------------------------------------------------------------------------ |
| Building blocks | `View`, `Text`, `Pressable`, for layouts the others don't cover                                              |
| Layout          | `Stack`, `Row`, `Card`, `Callout`, `Divider`, `Show`                                                         |
| Figures         | `Stat`, `Progress`, `Spinner`, `Chart` with `Series`, `DataTable`                                            |
| State           | `Var` (an input), `Let` (a formula)                                                                          |
| Input           | `Slider`, `Stepper`, `NumberField`, `TextField`, `Choice` (buttons or radios), `Toggle` (switch or checkbox) |
| Actions         | `Button`, `Pressable`                                                                                        |

`Pressable` makes a whole card or row one button, such as a card for each plan the user can pick. It holds text, figures, and layout, but not other buttons or inputs, because no platform lets one control sit inside another.

Charts draw bars, lines, areas, pies, and donuts, with series colours that stay distinguishable for colour-blind readers. Formulas cover arithmetic, comparisons, lists and records, totals and averages, running totals, compounding growth, and money and percentage formatting; the reference lists every function.

## What a document can't do

- Run code. Formulas are expressions with a fixed set of functions and a fixed amount of work each, so they always finish quickly.
- Change your data, or any variable it did not declare.
- Act on a button press. Presses reach your handler, which decides.
- Lay content over your interface. Classes such as `fixed`, `absolute`, `z-50`, and negative margins are refused, so a document can't draw a fake dialog over a real one. Scrolling classes are refused too: a document scrolls with your screen.
- Load an image or open a link. Images show their description unless you allow their address with `.allow_images(|url| url.starts_with("https://cdn.example.com/"))`, and links show as text.
- Grow without bound. `Limits` caps a document's size, elements, variables, list lengths, and chart points; past them, the rest is left out with a note.

## Platforms

A document renders on the web, iOS, Android, macOS, Windows, Linux, and in the terminal. Sliders, switches, checkboxes, radios, and spinners are the platform's own controls, so they feel native and work with screen readers. A slider stays under the user's finger while everything that reads its value follows.

Your users' installed app has to be able to draw those controls, and a code push can't change that: it updates your interface, not the native shell around it. Tell `AgentUi` what the shell draws:

```rust,ignore
use leptoskit::agent_ui::{AgentUi, Widgets};

// A macOS, iOS, or Android app generated before leptoskit 0.3.
let ui = AgentUi::new().widgets(Widgets::WithoutControls);
```

| Shell                                                                                               | Setting                      | What changes                                                                                                                                                                |
| --------------------------------------------------------------------------------------------------- | ---------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Web, Windows, Linux, terminal, and macOS, iOS, and Android apps generated by leptoskit 0.3 or later | `Widgets::All` (the default) | Nothing.                                                                                                                                                                    |
| macOS, iOS, and Android apps generated before leptoskit 0.3                                         | `Widgets::WithoutControls`   | A slider is a row of stops to tap, with − and + for single steps. Switches and checkboxes are buttons that show ☑ or ☐, radios are a row of buttons, and a spinner is text. |
| HarmonyOS (experimental)                                                                            | `Widgets::Basic`             | As above, and line and area charts draw as columns, pies and donuts as each part's amount and share, and a `Pressable` as a button with its label.                          |

The agent writes the same document for every shell, and the reference doesn't change.

- On the web and in the terminal, bold, italic, and link colour show inside a paragraph. The other platforms draw a paragraph in one style, with all of its words.
- In the terminal, bar charts draw in cells, while line, area, pie, and donut charts draw a placeholder texture; their legend and labels still show.
- Links and inline code take the light-appearance colour of their role, because a run of text has a fixed colour.
