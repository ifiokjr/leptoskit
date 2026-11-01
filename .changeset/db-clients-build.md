---
"release": patch
---

# Build apps whose client uses the database tables

A crate that depends on `leptoskit_db` without a database driver, such as an app's browser or native client sharing its tables with the server, stopped compiling with `unresolved import images`. It compiles again.
