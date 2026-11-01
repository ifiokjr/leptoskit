---
"release": minor
---

# `/asyncapi.json` describes the live operations

`leptoskit_rpc::spec::openapi_router` now also serves `GET /asyncapi.json`: an AsyncAPI 3.0 document built from the same spec. Queries are `receive` operations, mutations and actions are `send` ones, and each operation's arguments and result travel as message payloads with the existing JSON Schemas. Calls and internal functions are left out. `ApiSpec::to_asyncapi()` renders the document without a server, for tests and tooling.
