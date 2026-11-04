---
"release": patch
---

# Deliver a live socket's last frames before it closes

When the server closes a live WebSocket, such as one sending frames faster than it serves them, it now reads what the client is still sending until the client answers the close, for up to five seconds, before dropping the connection. Previously the connection could end in a reset that discarded the server's last frames, so on Windows a client lost the reason it was closed along with the close itself.
