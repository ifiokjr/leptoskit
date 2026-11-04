---
"release": minor
---

# Run your own deployment service

`lk console serve` runs one service, on a trusted Linux or macOS worker, that holds the deployment dashboard, a queue of deployment jobs, an API for agents, and the analytics history of every instance of your app. Connect an app once, and the ordinary commands run through it, so a deployment keeps going after you close your terminal:

```sh
lk console serve --project /srv/notes --url https://deploy.example.com   # on the worker
lk console connect --url https://deploy.example.com                       # in your checkout
lk infra plan
lk infra apply
lk deploy
```

It reuses the `lk infra` and `lk deploy` engine. Credentials stay on the worker and in a private file outside your repository. `apply` runs only the plan this checkout reviewed, and `lk deploy` builds the current Git commit in a temporary checkout on the worker. An operation interrupted by a restart is marked as such and waits for you to reconcile it, rather than being retried blind. `lk console token` issues project-scoped `read`, `deploy`, and `ingest` keys for agents and apps.

Apps with the analytics plugin get their ingestion key and endpoint automatically. The service keeps up to seven days of measurements per project, alongside process, cgroup, and filesystem samples from each host, and can export metrics to an OTLP/HTTP backend instead. `lk console backup` and `lk console restore` back up and restore the service's own database; a restore revokes the keys it brings back.
