---
"release": patch
---

# Preserve explicit line breaks in native text layout

Measure every newline-delimited line independently, retaining consecutive and trailing empty lines. Native layouts reserve their combined height while preserving word wrapping, font scaling and the widest line's width.
