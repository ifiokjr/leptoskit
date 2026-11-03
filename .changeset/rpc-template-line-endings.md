---
"release": patch
---

# Generate consistent RPC clients on Windows

Keep embedded RPC client templates in LF form on Windows checkouts. Generated clients now match their committed fixtures across platforms instead of inheriting CRLF lines from the generator's source templates.
