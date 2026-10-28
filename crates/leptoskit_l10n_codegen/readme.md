<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_l10n_codegen

> Reads a leptoskit `locales/` directory of ARB files and writes its typed translations module.

The `leptoskit l10n` command and the CLI's build pipeline use this crate; apps never depend on it.

- `generate(dir, template)` reads every `<tag>.arb` file, types each message's placeholders from the template's `@key` metadata and from how the message uses them, checks every translation against the template, and returns the generated module with a summary of each locale and message, or every problem with its file and line, sorted by both: files not named after a locale, JSON that does not parse, messages that are not well-formed, keys and placeholders that cannot become Rust names (keywords, `gen` included, and the methods the `L10n` handle answers to itself, such as `locale`), `=N` forms too large for their placeholder's type, placeholders whose types do not fit how they are used, and translations that do not match the template's placeholders.
- `write(output, &generated)` writes the module only when it changed, and never overwrites a file leptoskit did not write.
- `status(output, &generated)` compares without writing, for `leptoskit l10n --check`. Formatting and item order do not count as changes, so rustfmt can format the committed file.

Messages are the subset of ICU MessageFormat that ARB files use: text, `{placeholders}`, `{count, plural, ...}` with `=N` forms and `#`, `{kind, select, ...}`, nesting, and ICU's quoting, where quoting a placeholder (`'{name}'`) is an error rather than a silent literal. Locales with variants, such as `ca-ES-valencia`, each get their own messages. The generator never runs app code.
