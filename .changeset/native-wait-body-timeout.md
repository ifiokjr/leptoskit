---
"release": patch
---

# Time out native waits whose deadline expires mid-response

A native UI wait that runs out while reading a driver's response body now fails with the wait's `Error::Timeout`. Previously, reqwest reported that read's timeout as an I/O error, so a slow driver on a loaded host failed the wait with `request or response body error` instead of naming the condition that never appeared.
