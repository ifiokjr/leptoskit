---
"release": minor
---

# Add scoped asynchronous native plugin execution

Generated iOS, Android and macOS hosts can install HTTP and WebSocket plugins through validated, host-owned origin scopes. HTTP exposes bounded binary requests/replies; WebSocket exposes owned connection handles, bounded send/receive operations and explicit close. Redirects, destination scopes, deadlines and transport errors retain their documented platform behavior.

Add optional `async-host` dispatch with bounded request/reply reservations, cancellation, permission-revocation handling and resource cleanup. Native callbacks return owned replies to the bridge thread. Cancelled work retains its native job budget until cleanup completes. Unread WebSocket-open replies retain provisional resource ownership, so cancellation or teardown cannot orphan sockets.

The CLI enables async features only for selected native plugins. Interpreter clients use `default-features = false` and `guest`; browser clients retain their browser adapters. The default bridge has no Tokio runtime dependency. CI explicitly exercises async host and loopback network tests, Swift callback lifecycle, the ignored Kotlin/JNI lifecycle fixture under JVM checking, and guest dependency isolation.
