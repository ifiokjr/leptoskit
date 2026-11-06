<h1 align="center">leptoskit_agent_ui</h1>

<p align="center">
  <strong>leptoskit_agent_ui</strong> turns interfaces an AI agent writes while your app runs into native leptoskit elements: Markdown with components, variables, formulas, charts, and actions.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_agent_ui"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_agent_ui"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_agent_ui"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_agent_ui.svg"></a>
  <a href="https://docs.rs/leptoskit_agent_ui"><img alt="docs.rs" src="https://docs.rs/leptoskit_agent_ui/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

An agent answers in Markdown with components in it, the way MDX mixes them. `AgentUi` renders that text as an element tree on every platform. The agent can lay out text, show figures and charts, declare variables the user changes with steppers and fields, compute with formulas that follow those variables, and offer buttons whose presses come back to the application as `Action`s.

The document is data, never code. It can only use the components the application registered, call the built-in formula functions, and write the variables it declared. Formulas run on a fixed budget, documents are held to `Limits`, classes that would lay agent content over the application's own interface are refused, and images load only from addresses the application allows. A button does nothing by itself: the application's `on_action` handler decides.

Every built-in component renders to views, text, buttons, text fields, and vector paths, which every leptoskit host draws, including the Swift and Kotlin renderers in already-published apps. `AgentUi::reference` writes the language reference for the agent's instructions from the components registered right now, so a component added in a code push teaches the agent about itself in the same update.

## How it fits

<!-- {=agentUiSummary} -->

It interprets interfaces an agent writes as text while the app runs: Markdown with components, variables, formulas, charts, and actions. The documents are data, never code, so a published app shows new interfaces without a new build.

<!-- {/agentUiSummary} -->

## Use it

```rust
use leptoskit_agent_ui::AgentUi;

let ui = AgentUi::new()
    .data("income", 4200)
    .on_action(|action| println!("the user chose {}", action.name));

let rendered = ui.render(
    "<Var name=\"rent\" value={1800} />\n\n\
     <Stepper bind=\"rent\" step={50} label=\"Rent\" format=\"money\" />\n\n\
     Left over: {money(income - rent)}\n",
);
assert!(rendered.diagnostics.is_empty());
```

Show a streaming answer with `ui.view(move || answer.get())`, and send `ui.reference()` to the agent as its instructions.

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_agent_ui)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Interfaces from agents](https://leptoskit.dev/docs/guide/31-agent-interfaces.html)
