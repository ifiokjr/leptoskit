---
"release": minor
---

# RPC clients report schema fidelity loss

`leptoskit rpc generate` now warns, with the schema path and the target's generated fallback, wherever a client cannot carry a schema's shape: an arbitrary union or fixed tuple that collapses to `any`, an unresolved `$ref`, a multi-type or multi-member `allOf` schema, a non-string enum, nullable fields whose explicit null becomes absence, explicitly allowed extra object properties, and wide integers outside TypeScript or Dart runtime limits. Deliberately unconstrained JSON (`true`, `{}`, an untyped map or array) is reported as intentional, not lossy.

The default output is unchanged: every warning still generates the documented fallback. `leptoskit rpc generate --strict` fails before writing any file when any requested target loses structure, so a pipeline that needs exact types can refuse the fallback instead of discovering it in the generated code. `ApiSpec::diagnostics` exposes the same per-target report to callers of `leptoskit_rpc_codegen`. OpenAPI preserves schema nodes and does not run typed-client diagnostics. Strict mode detects known representation losses; it is not a general JSON Schema validator.
