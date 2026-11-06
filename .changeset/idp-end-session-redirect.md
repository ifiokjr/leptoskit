---
"release": patch
---

# Return from signing out only to the app that asked

`leptoskit_idp`: `/oauth/end_session` follows `post_logout_redirect_uri` only when the app named by the ID token in `id_token_hint` registered it, or, without an ID token, the app named by `client_id`. In every other case people land on your signed-out page, and `state` is dropped. That covers an unregistered address, a request that names no app, and a `client_id` that names a different app than the ID token. These requests used to get an error page. Now a valid ID token signs the person out in these cases too, where the error used to leave them signed in. A forged ID token is still refused.
