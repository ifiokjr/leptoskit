---
"release": minor
---

# Add leptoskit_form: headless form state

New `leptoskit_form` crate porting TanStack Form's core loop: declared `FieldSpec`s (label, help, initial value, `required`, ordered sync validators over the entered text) and a `FormState` whose edits arrive as `for_change` and `for_blur` proposals. A field's error first appears when the user leaves the field or submits the form, then re-resolves live on every edit, so a fix clears it at once and typing never raises one mid-word. `submit` runs every rule and returns the values or the error-filled state. Rendering goes through `leptoskit_aria::Field` wiring (`field_wiring`, `input`, `submit_button`), unstyled: ids are prefixed with the form's id so forms with the same field ids share a screen, labels name inputs, help and error ride `aria-describedby`, and values turn `aria-invalid` — announced by screen readers on the web and every native backend. A form rejects duplicate field ids. Re-exported behind the `leptoskit` `form` feature, which implies `aria`, and documented in a new forms guide.
