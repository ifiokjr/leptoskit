# leptoskit_docs_site

The binary that builds and serves the leptoskit documentation site at <https://ifiokjr.github.io/leptoskit/>.

The book itself is Markdown in [`docs/src`](../src), listed by `SUMMARY.md`. [`leptoskit_docs`](../../crates/leptoskit_docs) is the generic generator: it turns MDX-style Markdown (leptos components inside Markdown) into a static site. This crate is the leptoskit-specific part: the site's title, logo, favicon, social image, edit links, and stylesheet (`site()`), and the few components only these docs use (`components()`): `<Hero>` for the home page, `<Gallery>` with `<Shot>` for rendered screens, `<Swatches>` with `<Swatch>` for the brand colours, and `<Logos>` with `<Logo>`. The generator's built-in components, such as `<Callout>` and `<Steps>`, are available too.

The build is strict. A tag that is never closed, an unknown component or attribute, a broken or non-relative link, or a link to a missing heading fails it with the file, line, and column. [Writing the public docs](../agents/docs-site.md) lists what it rejects.

```sh
devenv shell docs:serve                 # serve on 127.0.0.1:4400 and rebuild and reload on every edit
devenv shell docs:build                 # build into docs/dist, the way CI publishes it
cargo run -p leptoskit_docs_site -- build --out docs/dist
cargo run -p leptoskit_docs_site -- serve --port 4400
```

`docs:build` first runs `schema:check` so the published JSON schemas cannot drift from the code. The `docs` workflow builds the site on every pull request that touches `docs/` or `leptoskit_docs`, verifies the published schemas match the checked-in ones, and deploys `docs/dist` to GitHub Pages from `main`. The integration test in `tests/build.rs` builds the real book and checks that every chapter in `SUMMARY.md` produced a page; run it with `cargo test -p leptoskit_docs_site`.
