---
"release": patch
---

# Close live connections that exceed their frame budget

Live WebSocket connections now close after sending the rate-limit error when their frame budget is exhausted. The regression test waits for both the error and connection termination, avoiding a race with the server's response under coverage instrumentation.
