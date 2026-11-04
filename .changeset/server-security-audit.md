---
"release": patch
---

# Close a channel authorization bypass and bound live connections

A live connection could join any channel instance by naming its key, even though the server only checked whether the caller could join the instance its arguments described. Members of one room could then see another room's presence and signals and send signals into it, including across servers through the relay. The server now requires the key to be the join arguments it authorized. Two channels whose arguments look alike (`Chat { room }` and `Doc { room }`) no longer share a room.

Live connections are also bounded in more ways:

- 64 channels per connection.
- Join arguments of at most 4 KiB, and presence state of at most 16 KiB.
- At most 16 actions running at once.
- A resume repeats each subscription only once.
- Frames that don't parse count against the frame budget.
- A WebSocket message far over the 1 MiB frame limit closes the connection before it is buffered.
- A client that stops reading is disconnected after 30 seconds (`Live::stall_timeout`), instead of making the server hold everything meant for it.

Reusing a subscription ID for another query no longer returns the earlier query's result when the caller's context runs alone.

`rpc::paginate` reads at most `rpc::MAX_PAGE_ROWS` (1,000) rows per page and `rpc::MAX_PINNED_ROWS` per pinned page, whatever `limit` the client sends.

`leptoskit_axum`'s servers now close a connection that hasn't finished sending its request headers within `HEADER_READ_TIMEOUT` (30 seconds). Without that timeout, a few slow clients could hold every socket the server had.
