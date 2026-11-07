# LeptosKit app

<!-- {=appSummary} -->

One `LeptosKit` component tree manages hosted installations from the web and native iOS/Android clients. The accompanying `leptoskit-cloud` binary hosts accounts, Stripe billing, per-account deployment journals, encrypted `OpenTofu` state and the worker API. Customer code runs on a dedicated outbound worker, using the existing CLI engine.

<!-- {/appSummary} -->

This is an operator-enrolled pilot. Customer-owned infrastructure works through a paired worker, hosted by the customer or LeptosKit. Managed DigitalOcean capacity uses an operator-provisioned installation with a reviewed quote, dedicated credentials and a separate worker. Cloud usage is separate from the management subscription. Provider account vending, automated usage invoicing and public self-service signup are not implemented.

The broker serves the public landing page at `/`, the management client at `/console/`, and the complete documentation book at `/docs/`, including search, deep links, theme switching and the configuration schema. The app's documentation action opens that same book in browsers or the system browser on native hosts. Stripe returns to `/console/`.

Build the complete website from the repository root with an installed `lk`:

```sh
cargo run -p leptoskit_app --features site --bin leptoskit-site -- \
  --url https://deploy.example.com --release
```

It writes `crates/leptoskit_app/dist/`. Pass `--cli /path/to/leptoskit` to use a locally built CLI, or `--out /path/to/artifact` to change the destination. The packager builds the browser Wasm client and the actual documentation book, expands the pinned Rive runtime, and writes the server-rendered public page. `lk build --platform web` alone builds the console client; it does not package the book. The broker checks required assets before accepting traffic.

The packager has two modes. The default packages the complete hosted artifact: the landing page, the management console entrypoint at `console/`, and the book under `docs/`, for the operator-hosted broker to serve from one origin. `--public-only` packages the public website for <https://leptoskit.dev>: the same landing page and live example, the book under `/docs/`, and the schema aliases at their stable `/schema.json` and `/schemas/` paths at the artifact root. It ships no console entrypoint, API, or SPA fallback — the public domain is static files only — and it removes a reused output directory's previous `console/` instead of leaving it behind. `--url` keeps setting every canonical link, so the hosted pilot keeps its own origin; the public packager runs with `--url https://leptoskit.dev`. The docs workflow publishes the public-only artifact; the pilot archive packages the complete one.

Native builds use `lk build --platform ios` or `android` from this directory. Set `LEPTOSKIT_APP_ORIGIN` at build time and replace the local HTTP origin allowlist in `leptoskit.toml` for production native builds. Browser clients use their own origin. Native account keys use the secure-store plugin; browsers retain them only in memory. The linking plugin opens documentation. Simulator/device validation and store publication are separate from compiling the native Wasm client.

The public page uses LeptosKit builders and an interactive Wasm counter. The website and book use the same skribble drawing-library approach as `kit.pina.rs`: seeded SVG frames and artwork, a light/dark paper palette and local Skribble lettering. The engine runs while packaging the book; the browser needs no drawing engine to show the ink. Fonts and the licensed Rive runtime are served locally. Every movement on the page is a LeptosKit `Motion` on its builders, which the web renderer writes as CSS animations and scroll-driven animations behind `prefers-reduced-motion` and `@supports`; no script animates the page. As the reader scrolls, the stage turns one component tree into each platform's widgets, and with reduced motion or without scroll timelines it shows every platform side by side. `web/landing.mjs` only defers the live example's Wasm client until the example nears the viewport or takes focus. The live example's mark uses the existing shared Rive integration. The public content and documentation remain readable without JavaScript. See [website design](../../docs/agents/website-design.md) for source pins and asset generation.

Build the broker from the repository root:

```sh
cargo build -p leptoskit_app --features server --bin leptoskit-cloud
leptoskit-cloud --data /srv/cloud --url https://deploy.example.com serve \
  --assets /srv/cloud-web --bind 127.0.0.1:4701
```

Copy the complete artifact to `/srv/cloud-web`; the default docs directory is `/srv/cloud-web/docs`. `--docs` can select a separately built book. Serve the complete book on the same origin for the supplied content security policy.

Check the live example's loader with `deno test crates/leptoskit_app/tests/landing.mjs` and the Rust integration with `cargo nextest run -p leptoskit_app --all-features`. After packaging, `LEPTOSKIT_SITE=docs/dist deno test --allow-env --allow-read --allow-net --allow-run --allow-write crates/leptoskit_app/tests/live_example.mjs` opens the artifact in headless Chrome (`CHROME` names another executable), presses the live example's button, and fails unless the count changes without a page error; the docs workflow runs it on every build. Read [the operator runbook](../../docs/agents/hosted-deployment-service.md) before configuring billing, enrolling accounts or providing paid infrastructure. The customer workflow is in [the hosted account guide](../../docs/src/guide/29-hosted-deployments.md).

For a Linux pilot, the [deployment and acceptance procedure](../../docs/agents/hosted-pilot-acceptance.md) packages the broker and complete website into one checksummed archive. The `hosted pilot` workflow tests that archive in a fresh Ubuntu container without network access, then publishes the candidate and its recovery report. Its systemd service listens on loopback, keeps data outside the release, and drains requests on termination. Cloud and Stripe acceptance remain separate credentialed steps.
