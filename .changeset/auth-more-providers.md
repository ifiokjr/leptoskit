---
"release": minor
---

# Sign in with Facebook, X, Discord, LinkedIn, GitLab, Slack

Six more providers have built-in definitions, each with a setup page and `leptoskit auth provider add <id>`:

- **Facebook**, through the Graph API version you pin with `api_version` (defaulting to `v26.0`); `leptoskit auth doctor` warns when yours is older.
- **X**, with OAuth 2 and PKCE, authenticating with HTTP Basic as X requires.
- **Discord**, trusting an address only when Discord's `verified` flag says so.
- **LinkedIn**, through "Sign In with LinkedIn using OpenID Connect".
- **GitLab**, on GitLab.com or a self-managed GitLab named with `issuer`.
- **Slack**, where a person is identified by their workspace and user together.

Facebook and X never vouch for an email address, so accounts made with them start without one, as with Microsoft.

A provider ID that names a built-in provider now uses its definition. If you configured one of these six as a generic OpenID Connect provider by ID, such as `[auth.providers.slack]` with an `issuer`, add `kind = "oidc"` to keep it that way; the configuration check says so.

The provider pages also now say what happens in native apps, which sign in through the system browser.
