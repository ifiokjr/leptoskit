---
"release": patch
---

# Sign-in starts on an IP-address origin, without passkeys

With the `passkey` feature on, `Auth` refused to start when `[auth] origin` was an IP address, such as `leptoskit dev`'s `http://127.0.0.1:3000`, because WebAuthn never accepts an IP address as a relying party. It now starts without passkeys there, and every other way to sign in works; asking for a passkey says why it's unavailable and suggests `http://localhost:3000`.
