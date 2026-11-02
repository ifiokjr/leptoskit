---
leptoskit_l10n_codegen: breaking
---

# leptoskit_l10n_codegen: the translations generator

`leptoskit_l10n_codegen` reads a leptoskit `locales/` directory of ARB files and writes its typed translations module.

The `leptoskit l10n` command and the CLI's build pipeline use it; apps never depend on it. It checks every translation against the template and reports placeholder, plural, and naming problems with a file and line.
