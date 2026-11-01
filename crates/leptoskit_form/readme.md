<h1 align="center">leptoskit_form</h1>

<p align="center">
  <strong>leptoskit_form</strong> is headless form state in the style of TanStack Form: typed fields, sync validation, touch and submit tracking, and inputs wired through the aria fields.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_form"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_form"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_form.svg"></a>
  <a href="https://docs.rs/leptoskit_form"><img alt="docs.rs" src="https://docs.rs/leptoskit_form/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Headless form state for the leptoskit element tree, ported from [TanStack Form](https://tanstack.com/form) at its core loop: a `Form` declares its `FieldSpec`s — labels, initial values, `required`, and validation rules over the entered text — and a `FormState` holds what the user did. Typing proposes `FormState::for_change`, leaving a field proposes `FormState::for_blur`, and `submit` runs every rule and returns the values or the error-filled state. A field's error first appears when the user leaves it or submits, then tracks every edit live, so a fix clears it at once and typing never raises one mid-word.

The rendering side is field wiring through `leptoskit_aria::Field`, the same contract as [leptos_aria](https://github.com/ifiokjr/leptos_aria): `field_wiring` prefixes its ids with the form's id so forms with the same field ids share a screen, names the input after its label, carries help and error through `aria-describedby`, and marks the value `aria-invalid` while an error shows — announced correctly by screen readers on the web and every native backend, with the renderer's validation holding the contract. `input` renders one field whole, validating when focus leaves it, and `submit_button` runs the rules on press; neither carries styling, so the app owns the look.

Use the crate directly or enable `leptoskit`'s `form` feature (which implies `aria`) and import `leptoskit::form::*`.

## How it fits

A form declares `FieldSpec`s once; the same state drives web inputs and native controls, and `leptoskit_aria` supplies the accessible field wiring. Validation and submission logic stay out of the view layer.

## Use it

```rust
use leptoskit_form::FieldSpec;
use leptoskit_form::Form;
use leptoskit_form::FormState;
use leptoskit_form::submit;

let form = Form::new("signup")
    .with_field(
        FieldSpec::new("email", "Email address")
            .required()
            .validate(|value| {
                (!value.contains('@')).then(|| "That address has no @.".to_owned())
            }),
    );

let state = FormState::initial(&form);
let rejected = submit(&form, &state).expect_err("a required field is empty");
assert_eq!(rejected.error_of("email"), Some("Email address is required."));

let state = state.for_change(&form, "email", "ifiok@example.com");
let values = submit(&form, &state).expect("a valid fill stands");
assert_eq!(values.get("email").map(String::as_str), Some("ifiok@example.com"));
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_form)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Forms](https://ifiokjr.github.io/leptoskit/guide/19-forms.html)
