---
"release": patch
---

# Kubernetes manifests that start

`leptoskit generate deploy --target kubernetes` now writes manifests a cluster accepts and runs:

- **Names:** Deployments, Services, Secrets, claims, and labels use the app's name with `_` as `-`, so an app called `my_app` gets `my-app`, a valid Kubernetes and DNS name. The image keeps its name.
- **The user:** each pod runs as the image's `app` user by number (`runAsUser`, `runAsGroup`, and `fsGroup` 10001). Before, the kubelet refused to start every container, because `runAsNonRoot` can't check a user named only by name. Volumes are now writable to that user on any storage class. The generated `Dockerfile` creates the `app` group with gid 10001 itself, rather than leaving the number to `useradd`.
- **Services without an app database:** a service with `database = true` in an app that has no `[db]` gets its SQLite file on a volume of its own under Compose and Kubernetes, with one replica, as it does when the app is on SQLite. Before, the file went on the read-only root filesystem, and Kubernetes gave the service no `DATABASE_URL`.

The analytics example's server now answers `/healthz` and `/readyz`, which every generated health check calls.
