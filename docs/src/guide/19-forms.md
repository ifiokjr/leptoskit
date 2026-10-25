# Forms

A form is field rules plus what the user did with them. `leptoskit_form` is that state, ported from [TanStack Form](https://tanstack.com/form) at its core loop, rendered through the [accessible components](16-accessible-components.md) field wiring: one form, correctly announced by screen readers on the web and every native backend.

```toml
[dependencies]
leptoskit_form = "0.1"
```

Or enable `leptoskit`'s `form` feature (which implies `aria`) and import `leptoskit::form::*`.

## Fields and state

A [`Form`][readme-form] holds typed [`FieldSpec`][readme-field]s — label, initial value, `required`, and validation rules — and is data, not state: build it once. `FormState` holds the values, the errors, and how far the user got. It never mutates itself; functions propose.

```rust,ignore
use leptoskit_form::{FieldSpec, Form, FormState, submit};

let form = Form::new("signup")
    .with_field(
        FieldSpec::new("email", "Email address")
            .with_help("We never share your address.")
            .required()
            .validate(|value| {
                (!value.contains('@')).then(|| "That address has no @.".to_owned())
            }),
    );

// An untouched submit shows every failing rule.
let state = FormState::initial(&form);
let rejected = submit(&form, &state).expect_err("a required email is missing");

// Typing proposes the next state; a touched field re-resolves as it changes.
let state = state.for_change(&form, "email", "ifiok@example.com");
let values = submit(&form, &state).expect("a valid fill stands");
```

Errors surface once a field is touched or the form is submitted — an untouched form is quiet, the way react-aria and TanStack Form surface them. `submit` runs every rule: `Ok` carries the values, `Err` carries the state with every error filled in and `submitted` set, ready to render.

## The rendering

`field_wiring` builds the leptoskit_aria [`Field`](16-accessible-components.md#field) for one field spec and state: the input is named after its label, help and error ride `aria-describedby`, and the value turns `aria-invalid` while an error shows. `input` renders one field whole — label, wired input, help, error — and `submit_button` runs the rules on press and hands the outcome to the screen. The renderer's own validation holds the contract: a wired field cannot ship half wired.

Build custom inputs from `field_wiring` when `TextInput` is not the control; everything the state produces is public.

## What comes next

Async validators, form-level rules across fields, and dirty tracking follow the same reducer pattern; server-side validation results merge through the same `errors` map.

[readme-form]: https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_form
[readme-field]: https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_form#fields-and-state
