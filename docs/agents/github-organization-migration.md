# Moving to the `leptoskit` GitHub organization

The project moves from `ifiokjr/leptoskit` to `leptoskit/leptoskit`. The organization exists so the plugin ecosystem can grow beside the core: MediaPipe, Firebase, analytics, notifications, and the server-side plugins each become their own repository rather than another folder here.

The public site is already served from <https://leptoskit.dev/>, so this move does not change the documentation URL or any published schema `$id`. The repository changes, the crates.io trusted publishers, and the organization settings land before the repository itself is transferred. The old publisher entries stay in place until a release from the new location has proved that the new ones work.

## What changes in this repository

| Where                                                                        | What changes                                                                 |
| ---------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| `Cargo.toml`                                                                 | `workspace.package.repository`; `homepage` stays `https://leptoskit.dev/`    |
| `monochange.toml`                                                            | `ecosystems.cargo.publish.trusted_publishing.repository`, and `source.owner` |
| `docs/site/src/lib.rs`                                                       | `REPOSITORY`; the site `URL` stays `https://leptoskit.dev/docs/`             |
| `.templates/badges.t.md`, every crate README, and every crate's `src/lib.rs` | Badge, repository, raw asset, and documentation links                        |
| `crates/leptoskit_cli/src/{skill,devtools/fetch}.rs`                         | The skill download and DevTools release URLs                                 |
| `crates/leptoskit_cli/src/verify/build.rs`                                   | The SLSA `buildType` and `builderId` identifiers, now under `leptoskit.dev`  |
| Snapshots, fixtures, and example manifests                                   | The `#:schema` header and repository links                                   |

Schema files and `leptoskit_config`'s `schema_url()` do not change: they already name `https://leptoskit.dev/`. The workflows do not name the repository; they use `$GITHUB_REPOSITORY`.

## Before the transfer

### 1. Create the organization

GitHub has no API for creating an organization. Create `leptoskit` in the web UI, then set the profile name, avatar, description, and website. The Free plan is enough: the repository is public.

The handle has to exist before crates.io can store a trusted publisher for it, because crates.io resolves the owner to a GitHub account ID when the publisher is saved.

### 2. Match the organization and repository settings

The current repository's relevant settings are:

- Actions: allow all actions and reusable workflows; default workflow permissions read and write; allow GitHub Actions to create and approve pull requests.
- Environments: `publisher`, with `ifiokjr` as a required reviewer, and `github-pages`. `publisher` is part of the trusted-publishing identity, so keep the name and the protection rule.
- Pages: source is GitHub Actions, and the custom domain is `leptoskit.dev`. Verify the custom domain and its DNS after the transfer; GitHub does not redirect Pages sites, and a lost custom-domain setting would publish the site at the GitHub Pages host instead.
- Merge settings: merge, squash, and rebase are all allowed; deleting the head branch on merge is off.
- Features: issues and wiki on; discussions off.
- Labels: `release` and `automated` are used by monochange, alongside the standard labels.
- Secrets, variables, rulesets, webhooks, and deploy keys: none today. A repository transfer keeps repository secrets, webhooks, and deploy keys associated, but verify the environments and Pages after the move.
- Security: secret scanning, push protection, Dependabot alerts, and private vulnerability reporting are worth enabling in the new organization.

### 3. Install the monochange GitHub App

The release workflow runs `monochange run release --commit --create-pr` with `MONOCHANGE_RELEASE_BACKEND: hosted`. The monochange GitHub App creates the verified release commit and the pull request. Install it on the `leptoskit` organization, grant it the new repository, and connect the repository in the monochange.dev dashboard. A repository that is not connected there cannot use the hosted backend.

### 4. Add the new crates.io trusted publishers

The new publisher is:

- Repository owner: `leptoskit`
- Repository name: `leptoskit`
- Workflow filename: `publish.yml`
- Environment: `publisher`

`setup/scripts/enroll_trusted_publishers.sh` discovers every publishable crate with `cargo metadata` and adds the configuration. It is additive: it skips a crate that already has this exact publisher and never changes or removes the `ifiokjr/leptoskit` publisher.

```sh
# A token with the `trusted-publishing` scope from https://crates.io/settings/tokens
export CRATES_IO_TOKEN=...

# Report first; this still reads the current publishers from crates.io.
setup/scripts/enroll_trusted_publishers.sh --dry-run

setup/scripts/enroll_trusted_publishers.sh
```

crates.io only accepts a trusted publisher for a crate that already exists. The script reports any crate that has never been published; publish that crate once with a token, then run the script again.

The repository currently has no `CARGO_REGISTRY_TOKEN` secret. If any crate is still unenrolled when the first release runs, the OIDC request falls back to that missing secret and the publish job fails. Either enroll every published crate, or add the secret to the new repository before the first release.

### 5. Keep the old Pages host as a courtesy

The canonical site and every schema URL already use `leptoskit.dev`, so the transfer breaks no schema consumers. The old `https://ifiokjr.github.io/leptoskit/` host will stop serving the site when the repository moves. If old inbound links should keep working, add a path-preserving redirect at `leptoskit/index.html` in `ifiokjr/ifiokjr.github.io`. This is optional and no longer required for configuration validation.

## The transfer

1. Transfer the repository:

   ```sh
   gh api --method POST repos/ifiokjr/leptoskit/transfer -f new_owner=leptoskit
   ```

   The UI path is Settings → General → Danger Zone → Transfer ownership. The account performing the transfer needs permission to create repositories in the organization.

2. Update every clone and worktree:

   ```sh
   git remote set-url origin git@github.com:leptoskit/leptoskit.git
   ```

3. Do not create a repository or fork at `ifiokjr/leptoskit`. Doing so permanently deletes GitHub's redirects for the old repository URLs, and old CLIs that download the DevTools archive or the skill from those URLs depend on them.

## After the transfer

1. Verify the `publisher` environment still exists and still requires review. Recreate it if the transfer did not carry it.
2. Verify the Actions settings, the Pages source, and the `leptoskit.dev` custom domain. A custom-domain setting lost in the transfer would publish the site at the GitHub Pages host instead. Confirm the docs workflow serves `https://leptoskit.dev/docs/` and the schemas at their existing paths.
3. Confirm the monochange app lists `leptoskit/leptoskit` as connected.
4. Run the release path once with a patch changeset: the release pull request, the tag, and `publish.yml` should all use the new identity. The publish job should report that it is using trusted publishing, not the token fallback.
5. Check that a crate's crates.io page now links to the new repository. Cargo metadata updates on the next publish; crates.io does not rewrite history.
6. Only after that release succeeds, remove the old `ifiokjr/leptoskit` trusted publisher entries from crates.io. Until then the old location remains a working publishing path, which is the point of adding the new publisher without removing the old one.

## What deliberately does not change

- The public site at `https://leptoskit.dev/`, including the `/docs/` paths.
- The configuration schema URLs, which already name `leptoskit.dev`.
- Crate names on crates.io and the `docs.rs` documentation URLs.
- The workflows' repository references; they read `$GITHUB_REPOSITORY`.
- The `ifiokjr/leptos_aria`, `ifiokjr/nixpkgs`, and Monosecret links in the documentation and development environment. They are other projects.
- Old release asset URLs already embedded in installed CLI binaries. GitHub redirects `github.com/ifiokjr/leptoskit/...` to the new repository as long as the old path stays unused.
