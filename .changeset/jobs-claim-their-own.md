---
"release": patch
---

# Workers claim only the jobs they can run

A server's job workers now take only jobs its own build declares. Before, during a rolling deploy, an old server could pick up a job the new build had just added, fail it with "no job is named …", and spend one of its attempts each time, until it gave up for good. Now the job waits for a server that knows it. A job no running build declares any more stays pending, where `leptoskit jobs list --state pending` shows it and `leptoskit jobs cancel` clears it.

Workers that had just started could also miss a job enqueued in their first moments and leave it until the next poll. They now look again once they're listening for new jobs.
