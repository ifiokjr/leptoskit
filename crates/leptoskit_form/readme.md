# leptoskit_form

Headless form state for the leptoskit element tree, ported from [TanStack Form](https://tanstack.com/form) at its core loop: a `Form` holds typed `FieldSpec`s — labels, initial values, `required`, validation rules — and a `FormState` holds what the user did. Typing proposes `FormState::for_change`; `submit` runs every rule and returns the values or the error-filled state. Errors surface once a field is touched or the form is submitted; an untouched form is quiet.

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

The rendering side is field wiring through `leptoskit_aria::Field`, the same contract as [leptos_aria](https://github.com/ifiokjr/leptos_aria): `field_wiring` names the input after its label, carries help and error through `aria-describedby`, and marks the value `aria-invalid` while an error shows — announced correctly by screen readers on the web and every native backend, with the renderer's validation holding the contract. `input` renders one field whole and `submit_button` runs the rules on press.

Use the crate directly or enable `leptoskit`'s `form` feature (which implies `aria`) and import `leptoskit::form::*`.
