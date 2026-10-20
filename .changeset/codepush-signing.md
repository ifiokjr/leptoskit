---
"release": minor
---

# Sign code push releases

`leptoskit codepush release` signs every release with an ed25519 publisher key, read from `--signing-key <PATH>` or the `LEPTOSKIT_CODEPUSH_SIGNING_KEY` environment variable. `leptoskit codepush keygen` creates the key and prints the public key for the new `codepush.public_keys` setting, which is required when code push is enabled.

`CodePushClient::new` takes a `TrustedKeys` set. Before downloading anything, the client rejects offers that are unsigned, signed by an untrusted key, tampered with after signing, meant for another app or channel, or not newer than the release it runs, so a compromised registry can neither ship its own code nor roll devices back. `ReleaseStore::ingest_directory` takes the signing key, and update offers carry the detached signature.
