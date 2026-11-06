---
"release": patch
---

# Launch the public website

`leptoskit-site` gains `--public-only`, the packaging mode behind <https://leptoskit.dev>. It produces the same polished landing page with its live Wasm component example, the complete documentation book under `/docs/`, and the configuration schema aliases at their stable `/schema.json` and `/schemas/` paths — as plain static files, with no console entrypoint, account API, or billing backend. The public page renders in a new public mode: navigation and calls to action lead to documentation and source, and the cloud section explains the CLI's deployment engine instead of offering the operator-enrolled pilot. The default packaging keeps building the complete hosted artifact with the console, so the pilot scripts and broker are unchanged; a reused output directory drops the previous console entrypoint when you switch to public-only.

The documentation book and the workspace now publish under the custom domain: the book's default URL and every brand and documentation link move from the old GitHub Pages host to `https://leptoskit.dev` (book pages under `/docs/`, schemas at their existing paths), and the docs workflow builds and deploys the real public-only artifact to GitHub Pages.

The live example's static button waits for the Wasm client to mount before accepting clicks, so the first interaction is not lost during loading.
