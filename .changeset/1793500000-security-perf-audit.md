---
"release": patch
---

# Faster web renders and reconciles; rate limits that run

A web re-render built every element's utility stylesheet twice — once only to hash it into a style scope — and hashed the result with SHA-256 on every rebuild, for every element, even when the document already carried the sheet. The scope now derives from the same content through the normalized class string, which determines the stylesheet, and the sheet itself is built only for a scope the document has not seen. Equal styles share one sheet exactly as before; only the scope names differ.

Reconciling a commit cloned every container's child list and deep-cloned every flattened node's props just to ask whether the node draws anything of its own, allocations on every frame's hot path. The child snapshot now lives in scratch space reused across commits, and the layout-only check compares fields in place — with the field list matched exhaustively, so a property added to `NativeProps` must be considered there to compile, and an unlisted one keeps its container a native view, the safe default. Production web builds also no longer format a devtools path for every node on every render when the devtools feature is off.

The server toolkit now serves with the peer address attached, so the auth layer's per-IP rate limits run in projects started from the scaffold: they read `ConnectInfo`, which plain `axum::serve` never fills in, and silently did nothing without it. Password reset also no longer runs Argon2 before the email code's rate limits have been paid for a guess. A channel index larger than its kind's bound is refused before it is parsed, a device remembers at most 64 rejected releases instead of every one forever, `leptoskit skill` refuses file names that would install outside its folder, a docs component link with a scheme a page cannot navigate to is a build error, and publishing to a plain-http S3 endpoint says what that exposes.

The wire error's constructors compile with the server's live module that calls them, so narrower builds — an app's server depending on `leptoskit_auth` alone — stay warning-clean too, and two stray debug prints are gone from the live client's tests.
