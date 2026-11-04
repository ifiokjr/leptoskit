---
"release": patch
---

# Package the hosted pilot for a Linux server

Build the broker, public site, console and documentation as one checksummed Linux release, with a systemd service that keeps persistent data separate from releases. Drain broker requests on termination so service restarts can finish active requests.
