---
"release": patch
---

# `<Status>` and `<Alert>` take their text as content

`view! { <Status>"Saved"</Status> }` built a live region with two text children, an empty one and "Saved", because the tag started from `Status("")`. It now builds the same tree as `Status("Saved")`: the region and one `Text`. `<Alert>` had the same problem and the same fix.
