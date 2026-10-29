---
"release": patch
---

# Clearer, consistent CLI output

Every command now prints through one palette, so the CLI reads as one tool:

- Errors print `error:` with the message, then each cause on its own `caused by:` line, instead of one long colon-joined line.
- Warnings say `warning:` in yellow; finished steps get a green `✔`, and steps with nothing to do a dimmed `○`.
- `leptoskit new` and `leptoskit generate` end with the commands to run next, in bold so they are easy to copy.
- `leptoskit doctor` and `leptoskit lint` close with an aligned summary table.
- `leptoskit dev` tags each line with its platform in its own color, and reports rebuild times as `1.2s` instead of `1.234567891s`.

Colors appear only on a terminal and respect `NO_COLOR`; piped output is plain text.
