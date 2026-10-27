# leptoskit_plugin_router

The file router as a leptoskit plugin: generates the typed `Route` module before cargo runs and answers `leptoskit plugin:router routes`. Apps depend on `leptoskit_router` for the runtime; this crate re-exports it alongside the generator, so tooling has one import point.
