---
"release": minor
---

# Require independent backend release approvals

Managed backend deployments can require independent signatures over the complete image descriptor and a current version-two deployment policy binding its exact digest and image identity. Reuse, rollback and recovery revalidate operator-owned trust and policy replay state without trusting stored verification flags.
