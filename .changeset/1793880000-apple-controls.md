---
"release": patch
---

# Render common controls in generated Apple apps

Generated iOS and macOS apps now render switches, checkboxes, radio options, sliders, progress bars, and activity indicators. Preserve their state through binary transactions, send toggle and value-change proposals to the guest, and expose native selection and value accessibility.

Tapping outside an input dismisses the keyboard while preserving button actions. Return ends editing and submits; guest-controlled focus can move to another field. Scroll views support interactive keyboard dismissal.

Focus requests on iOS no longer reopen the keyboard after dismissal on an unrelated render. macOS controls restore the state accepted by the guest after each proposal.
