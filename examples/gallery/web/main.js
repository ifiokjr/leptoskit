// Starts the gallery's Wasm client. The glue finds its module beside itself,
// and an external script keeps the page within a `script-src 'self'` policy.
import init from "./pkg/gallery.js";

await init();
