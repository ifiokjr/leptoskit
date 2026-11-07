// Starts the observatory's Wasm client. The glue finds its module beside
// itself, and an external script keeps the page within a `script-src 'self'`
// policy.
import init from "./pkg/observatory_example.js";

await init();
