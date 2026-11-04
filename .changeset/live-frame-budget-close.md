---
"release": patch
---

# Close live connections that exceed their frame budget

A live WebSocket connection that sends frames faster than the server serves them now receives the rate-limit error and is then closed. Before, it got the error but stayed open.
