---
"release": minor
---

# Keep composition helpers optional

Move the 31 layout and presentation builders into `components::helpers`, leaving native primitives as the primary `components` and prelude imports. Existing helper users should import the functions they need from `leptoskit::components::helpers` or `leptoskit_native::components::helpers`.

Build the shared component studio from `View`, Tailwind, and native controls. Document the React Native and Lynx primitive mappings separately from optional Flutter-inspired compositions.
