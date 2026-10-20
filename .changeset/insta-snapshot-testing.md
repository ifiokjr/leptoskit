---
"release": patch
---

# Pin generated CLI output with insta snapshot tests

The CLI's integration tests now capture everything it generates — scaffolded projects file by file, help text for every subcommand, error wording, and code push release artifacts including packed manifests — in committed insta snapshots, so any change to generated output arrives as a reviewable `.snap` diff instead of passing silently. Test projects moved into a `tests/fixtures` directory shared by the test suites, and the insta dependency gained the `filters` and `redactions` features for the `[ROOT]` path stand-ins and the manifest timestamp redaction.
