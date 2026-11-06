---
"release": patch
---

# Preview the deployment dashboard with example data

`leptoskit_deploy_console` keeps the dashboard's original design study beside the working deployment service. Its `model`, `store`, and `view` modules and its `preview` example render the dashboard's screens (deployment history, rollback and schema checks, deployment targets) from built-in example data, and export them as an interactive HTML page. The preview needs no server or credentials and never touches your cloud accounts; to deploy, run the service with `lk console serve`.
