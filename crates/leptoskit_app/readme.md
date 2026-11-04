# LeptosKit app

One LeptosKit component tree manages hosted installations from the web and native iOS/Android clients. The accompanying `leptoskit-cloud` binary hosts accounts, Stripe billing, per-account deployment journals, encrypted OpenTofu state and the worker API. Customer code runs on a dedicated outbound worker, using the existing CLI engine.

This is an operator-enrolled pilot. Customer-owned infrastructure works through a paired worker, hosted by the customer or LeptosKit. Managed DigitalOcean capacity uses an operator-provisioned installation with a reviewed quote, dedicated credentials and a separate worker. Cloud usage is separate from the management subscription. Provider account vending, automated usage invoicing and public self-service signup are not implemented.

The broker serves the public landing page at `/`, the management client at `/console/`, and the complete documentation book at `/docs/`, including search, deep links, theme switching and the configuration schema. The app's documentation action opens that same book in browsers or the system browser on native hosts. Stripe returns to `/console/`.

Build the complete website from the repository root with an installed `lk`:

```sh
cargo run -p leptoskit_app --features site --bin leptoskit-site -- \
  --url https://deploy.example.com --release
```

It writes `crates/leptoskit_app/dist/`. Pass `--cli /path/to/leptoskit` to use a locally built CLI, or `--out /path/to/artifact` to change the destination. The packager builds the browser Wasm client and the actual documentation book, expands the pinned Rive runtime, and writes the server-rendered public page. `lk build --platform web` alone builds the console client; it does not package the book. The broker checks required assets before accepting traffic.

Native builds use `lk build --platform ios` or `android` from this directory. Set `LEPTOSKIT_APP_ORIGIN` at build time and replace the local HTTP origin allowlist in `leptoskit.toml` for production native builds. Browser clients use their own origin. Native account keys use the secure-store plugin; browsers retain them only in memory. The linking plugin opens documentation. Simulator/device validation and store publication are separate from compiling the native Wasm client.

The public page uses LeptosKit builders and an interactive Wasm counter. Fonts and the licensed Rive runtime are served locally. The original brand animation uses the existing shared Rive integration; browser section entrances use progressive Web Animations. Both respect reduced motion and visibility. The public content and documentation remain readable without JavaScript. This does not add a native compiler for CSS or Tailwind keyframes.

Build the broker from the repository root:

```sh
cargo build -p leptoskit_app --features server --bin leptoskit-cloud
leptoskit-cloud --data /srv/cloud --url https://deploy.example.com serve \
  --assets /srv/cloud-web --bind 127.0.0.1:4701
```

Copy the complete artifact to `/srv/cloud-web`; the default docs directory is `/srv/cloud-web/docs`. `--docs` can select a separately built book. Serve the complete book on the same origin for the supplied content security policy.

Validate the motion policy with `deno test crates/leptoskit_app/tests/landing.mjs` and the Rust integration with `cargo nextest run -p leptoskit_app --all-features`. Read [the operator runbook](../../docs/agents/hosted-deployment-service.md) before configuring billing, enrolling accounts or providing paid infrastructure. The customer workflow is in [the hosted account guide](../../docs/src/guide/29-hosted-deployments.md).
