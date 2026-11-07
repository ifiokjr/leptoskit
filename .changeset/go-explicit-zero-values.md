---
"release": minor
---

# Go clients preserve explicit zero values

Generated Go clients use pointers for optional arguments and model fields, including slices and maps. Passing `nil` omits a field; passing a pointer to `false`, `0`, or an empty string includes that value. JSON and live calls also preserve empty collections; form and query encoding cannot express an empty collection. Previously `omitempty` could discard those values and cause the server to apply a default instead.

Regenerating changes Go source signatures: callers must pass addresses for optional values previously passed directly. The HTTP and live payloads share this representation. Optional nullable fields still have the null-versus-absence limitation reported by strict schema diagnostics.
