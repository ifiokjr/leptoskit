---
"release": patch
---

# Allow hosted accounts to subscribe again after cancellation

Bind replacement Checkout completion to the pending reservation before selecting the new subscription. Delayed events from the old subscription no longer reject the new one or clear its pending checkout, and repeated deliveries remain idempotent.
