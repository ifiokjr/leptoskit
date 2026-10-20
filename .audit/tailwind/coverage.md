# CSS coverage audit

Grounding uses the W3C CSS Snapshot (https://www.w3.org/TR/CSS/) and Tailwind utility documentation (https://tailwindcss.com/docs/). CSS is independently versioned modules, not a single layout specification.

The implementation must classify each module as layout, rendering, text, interaction, authoring/cascade, or browser/document behavior. Declaring a field is not implementation: native measurement, frames, appearance, events and web output need evidence.

Initial findings: flex and grid exist; content-box, signed margins, relative inset precedence and fractional minmax tracks require corrections. Existing visual props cover color/background, uniform radius/border, opacity, font size/weight/alignment. Native/web shared authoring is missing. Baselines: 48 Yoga tests, 19 native renderer tests and two wire tests pass on original main (cargo test -p leptoskit_yoga -p leptoskit_native --all-features).
