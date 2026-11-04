# Hosted pilot deployment and acceptance

This procedure deploys the operator-enrolled hosted broker, public website, console and documentation to one Ubuntu 24.04 host. It uses a short supervised restart for broker updates. Application deployments on dedicated workers retain their separate release behavior.

The public endpoint, a working broker, and payment acceptance are separate results. Packaging and local fixtures do not establish that a cloud deployment or Stripe payment succeeded. Keep an acceptance record with the source revision, artifact digest, public origin, provider resource IDs and each completed check. Keep credentials, account keys and state exports outside that record and outside Git.

## Choose the pilot target

Confirm the public hostname, its Cloudflare account/zone, an existing Ubuntu host or permission to create one, the provider region and spending limit, and an administrator SSH route. Price the broker, dedicated worker, test application's compute, storage and backups separately. A compute estimate is not a provider billing cap. Do not give the shared broker provider credentials or install customer workers beside it.

Start with a Stripe sandbox. Test-mode subscriptions do not move funds; they do not establish live-mode payment acceptance. Use the [Stripe testing guide](https://docs.stripe.com/testing) and [Billing test scenarios](https://docs.stripe.com/billing/testing). A live launch needs a separate decision about live keys, products, pricing and merchant readiness.

## Build a candidate

Run the `hosted pilot` workflow manually with the real `public_origin`, from the trusted commit being deployed. Pull requests build a candidate for `https://pilot.example.test`; those artifacts are for validation, not the public deployment. The workflow does not provision resources or use payment credentials.

The workflow builds with the pinned Rust toolchain and Ubuntu's compiler. The browser optimizer is Binaryen 132, with its release archive checksum pinned. It packages the native Linux broker and the complete site, then extracts the archive and verifies `SHA256SUMS`. A fresh Ubuntu container runs the candidate as an unprivileged user with a read-only filesystem, temporary writable data and no external network. The drill checks the actual HTTP routes, inactive-account policy, graceful termination, account persistence over restart, offline backup and restored-key revocation. Systemd syntax is checked separately. The uploaded artifact contains the archive and a JSON acceptance report, retained for seven days.

To build on an Ubuntu 24.04 development host instead, use a clean committed checkout with the system compiler, Python 3.11+, `readelf`, Rust targets and `wasm-opt` installed:

```sh
bash setup/scripts/package_hosted_pilot.sh https://pilot.example.com target/hosted-pilot
```

The builder refuses existing output, non-Linux hosts, invalid HTTPS origins, dirty source trees and a broker linked to the Nix store. `release.json` records the commit, origin, native target and build OS. The archive contains `leptoskit-cloud`, `site/`, `systemd/`, the environment example and checksums. It contains no database or configured credentials. Verify that its target matches the destination host. The supplied workflow produces x86-64 GNU/Linux artifacts for Ubuntu 24.04; it does not claim compatibility with older glibc hosts.

## Install on the approved host

Transfer the archive over the approved SSH route. Extract it into a new root-owned release directory, for example `/opt/leptoskit-cloud/releases/COMMIT`, and run `sha256sum --check --strict SHA256SUMS` there. Verify `release.json` against the approved commit and origin. Install `ca-certificates` and `libstdc++6` on the host.

From the extracted release directory:

```sh
sudo install -m 0644 systemd/leptoskit-cloud.sysusers /usr/lib/sysusers.d/leptoskit-cloud.conf
sudo systemd-sysusers /usr/lib/sysusers.d/leptoskit-cloud.conf
sudo install -m 0644 systemd/leptoskit-cloud.service /etc/systemd/system/leptoskit-cloud.service
sudo install -d -m 0700 /etc/leptoskit-cloud
sudo systemctl daemon-reload
```

Create `/etc/leptoskit-cloud/broker.env` as root-owned mode `0600`, following `broker.env.example`. Set the exact public origin, a persistent random encryption key of at least 32 characters, the Stripe sandbox key, this endpoint's webhook signing secret, and two distinct recurring Price IDs. Back up the encryption key separately from the database. The environment file is read by systemd; it is not a shell script. Never put real values in the source example or release archive.

Point `/opt/leptoskit-cloud/current` to the new release directory. Make the root-owned release files readable by the service account; public assets are mode `0644`, directories and the broker executable are mode `0755`. Enable and start `leptoskit-cloud.service`. Systemd creates `/var/lib/leptoskit-cloud` with mode `0700`; that persistent directory stays outside releases. The broker binds only `127.0.0.1:4701`. Check `curl --fail http://127.0.0.1:4701/healthz` and the service journal before enabling public routing.

For an update, stop the service, take an offline backup into a new private directory, change the release symlink, start the service and probe it. Keep the previous release and backup. This is one serving broker per data directory, not overlapping broker replicas. A failed candidate does not authorize automatic database reversal: inspect its migration compatibility and use the documented restore procedure when required.

## Route the hostname through Cloudflare

Create a dedicated remotely managed tunnel and route only the approved hostname to `http://127.0.0.1:4701`, with a final catch-all returning 404. Create the proxied DNS record for that tunnel. The host needs outbound tunnel connectivity; the broker needs no public inbound application port. Retain only the approved administrator SSH access.

Run `cloudflared` under its own service identity. Store its tunnel token in a private file outside releases and use `cloudflared tunnel run --token-file PATH`, supported since cloudflared 2025.4.0. Follow the current [Cloudflare tunnel setup](https://developers.cloudflare.com/tunnel/get-started/) and [run parameters](https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/configure-tunnels/run-parameters/). Keep Cloudflare account management tokens off the broker host. Use the intended edge rate limits, and keep the signed Stripe webhook route reachable without an interactive login challenge.

Probe the deployed hostname from outside the host:

```sh
python3 setup/scripts/hosted_pilot_smoke.py https://pilot.example.com
```

This checks the website, console, deep documentation/search, fonts and Wasm media types, CSP, redirects, missing routes, account authentication and unsigned webhook rejection. It leaves cloud-worker and Stripe payment acceptance marked pending. Also verify the actual browser counter, Rive scene, documentation navigation and console billing controls at the public origin.

## Accept Stripe sandbox billing

1. Verify that the API key and every customer, Price and webhook endpoint belong to the sandbox. Create dedicated pilot objects with an identifiable acceptance-run label. Record only their non-secret IDs. Configure the customer portal for those prices.
2. Register a public HTTPS snapshot webhook at `/api/v1/webhooks/stripe`, using the API version in the broker (`2026-10-28.endive`). Subscribe to `checkout.session.completed` and `customer.subscription.created`, `customer.subscription.updated` and `customer.subscription.deleted`. Install that endpoint's signing secret, then restart the broker. The [webhook guide](https://docs.stripe.com/webhooks) describes real delivery and signature verification.
3. Enroll a dedicated sandbox customer with the broker's `account` command. Run operator commands as `leptoskit-cloud` with the same environment file, data path and URL. For example, `sudo systemd-run --wait --pipe --uid=leptoskit-cloud --property=EnvironmentFile=/etc/leptoskit-cloud/broker.env /opt/leptoskit-cloud/current/leptoskit-cloud --data /var/lib/leptoskit-cloud --url https://pilot.example.com account --id pilot-acceptance --customer cus_REPLACE` prints the account key. Capture it only in the private operator session. It must start inactive and be refused permission to create installations.
4. Sign in to `/console/`, choose a plan and complete the hosted Checkout using Stripe's interactive test values. Repeating Checkout before completion must reuse the existing reservation. Confirm Stripe delivered a real signed event successfully, the account became active with the selected plan, and the return URL uses `/console/`. Creating a Checkout URL alone is not successful payment acceptance.
5. Open the customer portal. Exercise a plan change and cancellation; confirm authoritative provider state reaches the broker and the configured entitlement policy is enforced. Include cancel-and-resubscribe, duplicate delivery and a delayed old event in the acceptance record. Use the Stripe CLI's event resend or test-clock facilities where appropriate; do not load test Stripe.
6. Stop and restart the broker, then confirm the subscription and account remain consistent. Test a forged event separately; it must not change entitlement. A payment provider failure must not grant access or delete running cloud resources or state.

Record Checkout, portal, webhook and entitlement results individually. The repository's fake-Stripe tests and local packaging drill remain separate evidence. Do not report live-mode charging as tested by a sandbox run.

## Accept one dedicated cloud worker

Use the [hosted customer workflow](../src/guide/29-hosted-deployments.md) with a disposable application repository and a dedicated worker, provider credential boundary and infrastructure prefix. Include all planned resources in the approved spend. Keep the shared broker out of that execution environment.

Pair the one-use grant through the worker's secret store, confirm a second redemption is rejected, and run `lk console worker`. Exercise status, infrastructure plan, reviewed apply, first release, readiness, a subsequent release and compatible rollback. Confirm operations and encrypted HTTP state survive broker restarts, and that another account cannot read their results. Verify application traffic at its real public hostname and verify an off-site backup can restore to a replacement target.

Interrupt the worker only on this disposable installation. Preserve its pending marker, confirm automatic replay is refused, revoke the old worker, inspect actual provider and release state, and reconcile before retrying. Review retained state locks explicitly. Resource deletion must follow the approved teardown plan after exporting the acceptance evidence and retaining any required backups; a failed test alone does not authorize deleting unrelated resources.

The pilot is ready only when the credentialed cloud, billing and recovery steps have recorded results. DigitalOcean resale approval, usage invoicing, public signup, native device validation and store publication retain their existing separate boundaries.
