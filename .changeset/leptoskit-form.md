---
"release": minor
---

# Add leptoskit_form: headless form state

New `leptoskit_form` crate porting TanStack Form's core loop: typed `FieldSpec`s (label, help, initial value, `required`, ordered sync validators) and a `FormState` whose edits arrive as `for_change` proposals that re-resolve a touched field's error. `submit` runs every rule and returns the values or the error-filled state; errors surface only after touch or submit. Rendering goes through `leptoskit_aria::Field` wiring (`field_wiring`, `input`, `submit_button`): labels name inputs, help and error ride `aria-describedby`, and values turn `aria-invalid` — announced by screen readers on the web and every native backend. Three behavior tests plus doctests; a new forms guide; re-exported behind the `leptoskit` `form` feature, which implies `aria`.
