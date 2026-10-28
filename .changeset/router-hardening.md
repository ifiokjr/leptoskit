---
"release": patch
---

# Route every URL and every file name safely

`Route::parse` now reads URLs the way a browser does, so it never fails: a stray `%` stays as it is, escapes that aren't UTF-8 become U+FFFD, and `.` and `..` segments are resolved. `Navigation::from_browser()` mounts on any URL, and `ParseError` and `UrlError` are gone; `parse` and `from_browser` return `NoMatch`. Only path values decide whether a route matches: a query value that doesn't parse is ignored, so `/notes/42?highlight=x` is note 42, and a path parameter never reads from the query, so `/about?lang=fr` has no language.

An href never holds an empty, `.` or `..` segment and never starts with `//`: an optional segment holding one is left out, and a required segment or catch-all value holding one ends the path, so an empty value can no longer produce a link to another site.

The generator turns every file name into valid Rust, escaping modules and variants the same way (`404.rs` is `_404`, `self.rs` is `self_` and `Self_`, `gen.rs` is `gen_`), and reports a problem it cannot fix instead of panicking. A route with optional segments is ordered once for each combination of them present and absent, so `/about` reaches `[[lang]]/about.rs` before `[section].rs`, and two routes that match the same URLs through their optional segments are reported. A `_layout.rs` without a `layout` is an error, private helpers named like exports are ignored, `home.ios.rs` and the other `platform_module!` suffixes are recognised as platform overrides, and route struct fields are classified exactly as `#[derive(Route)]` reads them.
