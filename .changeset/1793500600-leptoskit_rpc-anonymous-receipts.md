---
"release": patch
---

# Anonymous callers' repeated mutations run once

Anonymous callers' receipts used to be owned by empty text. Under row-level security, that read as no owner, so their mutations with a request ID were refused. Their receipts now belong to `receipts::ANONYMOUS`.
