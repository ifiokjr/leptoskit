---
"release": minor
---

# Manage deployments from a hosted account

A hosted LeptosKit account keeps deployment requests, application measurements, and OpenTofu state in one place, managed from one app that runs on the web, iOS, and Android. Accounts are created by the service's operator, and subscriptions go through Stripe Checkout and the Stripe customer portal.

Deployments run on a dedicated worker that connects out to the service, so it needs no public address. Pair it with a one-use code from the app, then use the usual commands:

```sh
lk console worker --url https://deploy.example.com --project /srv/notes --data /srv/notes-worker
lk console connect --url https://deploy.example.com
lk deploy
```

Each account's data and keys are kept apart, and no two customers share a worker. With `infra.hosted_state = true`, OpenTofu state is stored encrypted, versioned, and locked by the service. Agent keys can deploy but can't manage billing, pair workers, or apply infrastructure changes. Workers run on your own infrastructure, or the operator can provision a dedicated DigitalOcean installation for you after reviewing its configuration and cost. A worker that stops partway through an operation doesn't repeat it automatically; it waits for the operator to reconcile it.
