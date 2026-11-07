# leptoskit_docs_site

The binary that builds and serves the leptoskit documentation site at <https://leptoskit.dev/docs/>.

The book itself is Markdown in [`docs/src`](../src), listed by `SUMMARY.md`. [`leptoskit_docs`](../../crates/leptoskit_docs) is the generic generator: it turns MDX-style Markdown (leptos components inside Markdown) into a static site. This crate is the leptoskit-specific part: the site's title, logo, favicon, social image, edit links, and stylesheet (`site()`), and the few components only these docs use (`components()`): `<Hero>` for the home page, `<Gallery>` with `<Shot>` for rendered screens, `<Play>` for an example's playable web build, `<Swatches>` with `<Swatch>` for the brand colours, and `<Logos>` with `<Logo>`. The generator's built-in components, such as `<Callout>` and `<Steps>`, are available too.

The build is strict. A tag that is never closed, an unknown component or attribute, a broken or non-relative link, or a link to a missing heading fails it with the file, line, and column. [Writing the public docs](../agents/docs-site.md) lists what it rejects.

```sh
devenv shell docs:serve                 # serve on 127.0.0.1:4400 and rebuild and reload on every edit
devenv shell docs:build                 # build into docs/dist, the way CI publishes it
cargo run -p leptoskit_docs_site -- build --out docs/dist
cargo run -p leptoskit_docs_site -- serve --port 4400
```

## Players

`<Play app="gallery" start="?screen=chat" title="Chat" description="…" native="…">` frames an example's real web build on a page. `src/play.rs` lists the examples the docs play; the `leptoskit-site` packager builds each one with `leptoskit build --platform web --release` into `play/<name>/` beside the book, after the book itself, and points the build's `<base href="/" />` at where it is served. `theme/play.js` loads each frame as it nears the viewport and scales the app, laid out at `width`×`height` (390×844 unless set), to the column; `theme/play-frame.js` runs first inside the frame and reports to the player over `postMessage`. A frame is sandboxed with `allow-scripts` alone, or with `allow-same-origin` as well for an example that routes, because Safari lets a framed page change its path only when it keeps its origin.

`docs:serve` and `docs:build` build the book alone, so their players say the example isn't built and switch to the native render. To play them, package the site, then serve `docs/dist` the way `crates/leptoskit_app/tests/browser.mjs` does: `docs/site/tests/playable_examples.mjs` opens every player in headless Chrome and plays it.

`docs:build` first runs `schema:check` so the published JSON schemas cannot drift from the code. The `docs` workflow builds the site on every pull request that touches `docs/` or `leptoskit_docs`, verifies the published schemas match the checked-in ones, and deploys `docs/dist` to GitHub Pages from `main`. The integration test in `tests/build.rs` builds the real book and checks that every chapter in `SUMMARY.md` produced a page; run it with `cargo test -p leptoskit_docs_site`.
