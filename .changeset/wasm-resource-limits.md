---
"release": patch
---

# Bound native Wasm memory and tables

Every native Wasm host now limits its guest to one 64 MiB linear memory and two tables with at most 65,536 elements each. The limits apply during loading and every later callback, including unexported resources and the module's start function. Oversized allocations and growth beyond these budgets return `WasmError::ResourceLimitExceeded`.
