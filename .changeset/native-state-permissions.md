---
"release": minor
---

Add application identity, localization snapshots, network state/change observation, validated outbound links and camera/microphone permission flows as separate plugin crates. Register Apple and Android handlers from metadata, preserve unknown platform fields, and retain permission-request identity across Android Activity recreation. Incoming deep links and additional permission families remain unsupported.
