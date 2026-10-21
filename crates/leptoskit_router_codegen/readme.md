<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_router_codegen

> Reads a leptoskit routes directory and writes its typed `routes.rs`.

The `leptoskit routes` command and the CLI's build pipeline use this crate; apps never depend on it.

- `generate(dir)` reads the routes directory and returns the generated module with its route table, or every problem with its file and line: names that aren't routes, route files without a page, route structs that don't match their path, and routes that compete for the same URLs.
- `write(output, &generated)` writes the module only when it changed, and never overwrites a file leptoskit didn't write.
- `status(output, &generated)` compares without writing, for `leptoskit routes --check`. Formatting and item order don't count as changes, so rustfmt can format the committed file.

The generator reads file names and the names of top-level items with `syn`. It never runs route code.
