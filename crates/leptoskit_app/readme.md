# LeptosKit app

One LeptosKit component tree manages hosted installations from the web and native iOS/Android clients. The accompanying `leptoskit-cloud` binary hosts accounts, Stripe billing, per-account deployment journals, encrypted OpenTofu state and the worker API. Customer code runs on a dedicated outbound worker, using the existing CLI engine.

This is an operator-enrolled pilot. Customer-owned infrastructure works through a paired worker, hosted by the customer or LeptosKit. Managed DigitalOcean capacity uses an operator-provisioned installation with a reviewed quote, dedicated credentials and a separate worker. Cloud usage is separate from the management subscription. Provider account vending, automated usage invoicing and public self-service signup are not implemented.

Build the app from this directory with `lk build --platform web`; it writes `dist/`. Native builds use `lk build --platform ios` or `android`. Set `LEPTOSKIT_APP_ORIGIN` at build time and replace the local origin allowlist in `leptoskit.toml` for production native builds. Browser clients use their own origin. Native account keys use the secure-store plugin; browsers retain them only in memory. Simulator/device validation and store publication are separate from compiling the native Wasm client.

Build the broker from the repository root:

```sh
cargo build -p leptoskit_app --features server --bin leptoskit-cloud
```

Read [the operator runbook](../../docs/agents/hosted-deployment-service.md) before configuring billing, enrolling accounts or providing paid infrastructure. The customer workflow is in [the hosted account guide](../../docs/src/guide/29-hosted-deployments.md).
