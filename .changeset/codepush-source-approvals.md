---
"release": minor
---

# Require independent source approvals for code push releases

Add `[codepush.provenance]` so native apps install a code push release only when enough independent approvers say they rebuilt it from one commit of a public GitHub repository and got byte-identical files. Approvals are a second trust root, separate from the publisher keys: each is signed by an approver over its exact statement, under its own signing context, so a publisher signature can't stand in for one. Devices check approvals before downloading a release and again before every launch, refuse approvers that name different commits, count each approver once, and ignore approvals from any other key, including the publisher's. The bundle shipped in the store build needs none, and without the table nothing changes.

Add `leptoskit codepush approve`, which signs an approval only when a rebuilt directory holds exactly the release's files, `leptoskit codepush attach`, which adds approvals to stored releases without re-signing them, and `leptoskit codepush keygen --approver`. `publish` and `serve` carry each release's approvals at `approvals/<bundle hash>.json`. An approval says who rebuilt identical files from a commit; it is not a security review.
