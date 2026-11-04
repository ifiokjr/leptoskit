# Manage deployments through a hosted account

Use a hosted LeptosKit account to keep deployment requests, application measurements and OpenTofu state in one place. The LeptosKit app uses the same screens on the web, iOS and Android. The website connects to its own server; native builds connect to the server configured when the app is built.

This release supports operator-enrolled accounts. Your service operator supplies the URL and initial account key. It does not create an account on a public LeptosKit service automatically.

Open your service's `/console/` page to manage installations. Its public homepage links to the complete documentation at `/docs/`. Choose **Documentation** in the management app to open the same book; on iOS and Android it opens in the system browser. Guides, platform support details, search and the configuration schema are packaged with the website.

## Connect your own infrastructure

For example, register a production app named `notes-production` in the management app. Choose the subscription shown in Checkout, then add an installation with that project ID. The ID must match your app's `infra.name`.

Keep the existing infrastructure settings and add:

```toml
[infra]
# Other provider, domain, image and storage settings go here too.
name = "notes-production"
control_url = "https://deploy.example.com"
hosted_state = true
```

`hosted_state` selects the hosted HTTP state backend. Provider credentials stay on a dedicated worker. If this app already has local or S3 state, stop and migrate that state deliberately before changing backends; changing this setting does not move existing resources or state.

Choose **Pair worker** in the app. Give the one-use code to your worker through its secret store as `LEPTOSKIT_PAIRING_CODE`, then start:

```sh
lk console worker --url https://deploy.example.com --project /srv/notes --data /srv/notes-worker
```

The worker needs the same Docker, OpenTofu, Kamal, Git, SSH and provider credentials as a [managed deployment](24-deploying.md). It connects outward and needs no public listener. Your operator can host this dedicated worker for you. Its repository and credentials belong to one installation's trust boundary.

From your development checkout, supply the account key through `LEPTOSKIT_CONTROL_KEY` and connect:

```sh
lk console connect --url https://deploy.example.com
lk infra plan
lk infra apply
lk deploy --setup
```

Review the plan before applying. Push your commit to the worker's registered repository origin before deploying it. Later releases use `lk deploy`; choose **First release** or **Maintenance** in the app when needed. The app can plan, apply a reviewed plan, deploy a version and roll back a compatible image. Refresh to read queued and completed operations. A failed connection does not cancel a queued deployment.

## Keys and state

Use **Create agent key** to issue a project-scoped deployment key. Store it as a secret. It cannot manage billing, pair workers or apply provider changes. The CLI can also issue read and ingestion keys with `lk console token`. Keys use the account's namespace so requests reach the correct journal. Revoke an agent key through the service's token API; revoke a worker from its installation card.

The worker stores its replacement credential in its private data directory. A pairing code expires after ten minutes and works once. Browser account keys are kept in memory; native clients use the operating system's secure store. Signing out clears the app's local credential. Ask the operator to rotate a lost account key.

OpenTofu state is encrypted on disk, isolated by account and project, and versioned. Writes require the current lock ID. The account owner can export the latest state through an authenticated GET to `/api/v1/state/PROJECT`; store that export securely because state may contain provider secrets. A subscription problem does not delete state, running applications or release history.

## LeptosKit-managed capacity

Request a dedicated DigitalOcean installation from the service operator. The operator reviews the infrastructure configuration and quote, sets spending limits, provisions the dedicated worker and enables the installation. The same app and CLI then manage its releases. Customer-owned installations and LeptosKit-managed installations are labeled separately in the app.

Your subscription pays for management. Compute, storage, transfer and other provider usage have a separate quote and invoice. The current implementation does not automatically meter or invoice that usage, rent model instances, or sell an unlimited infrastructure allowance. Checkout displays the configured subscription price; the toolkit does not fix a universal monthly price.

## Interrupted operations

A worker writes a private pending marker before execution and saves its result before acknowledging completion. If it restarts after saving the result, it retries the acknowledgement. If it stops during execution, it refuses to repeat the operation automatically. The operator must revoke the old worker, inspect the actual infrastructure and shared release ledger, reconcile the hosted operation, and then repair its pending marker before re-pairing.

Image rollback still checks the actual database schema. Database reversal remains a separate reviewed operation on the dedicated worker, with its existing backup and confirmation requirements. See [database migrations](22-database.md) and [your deployment service](28-deployment-service.md).
