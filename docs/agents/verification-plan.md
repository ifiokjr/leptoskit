# Full-stack verification

The goal is to let an independently installed verifier connect a public source revision to the frontend it runs and to the backend serving its actual connection. A copied badge, a version endpoint, an image tag, or a successful cluster scan must not qualify as verified execution.

Status: approved direction, staged implementation. Implemented: the **offline backend bundle inventory** (`verify record`/`check`) and the **reproducible backend builder** (`verify build`/`reproduce`), both described below. Signed build provenance, runtime images, runtime attestation, response receipts, and verified frontend execution in browsers are not implemented. The proposed runtime reference is Contrast confidential containers; no hosting provider has been selected or provisioned.

## Claims and trust boundaries

Keep independently checkable claims separate:

1. **Bundle integrity:** these extracted files match an independently pinned inventory.
2. **Source and build provenance:** a recognized builder used these exact source and dependency inputs, with this recipe, to produce these artifacts.
3. **Independent reproduction:** an independent execution of that recipe produced the same executable bytes.
4. **Backend connection:** the peer holding this connection key is an authorized protected runtime enforcing this approved release and workload policy.
5. **Frontend execution:** a separately trusted loader verified the executable resources before running them and enforces backend connection verification.
6. **Audit coverage:** identified reviewers examined the exact source/materials or deployment policy named by their signed reports. A report does not prove that the code is safe.

A JSON field declaring a repository, builder, secret name, or image digest is input to verification, not proof that the declaration is true. Never promote a claim based on metadata supplied solely by the application being checked. Establish the expected project identity and accepted builder/verifier roots independently. An official deployment and another deployment of the same public code must have distinct authorized identities.

The attacker may control the website, CDN, load balancer, registry, Kubernetes API, host administrator account, ordinary server signing keys, and one replica among many legitimate replicas. They can selectively target a user, serve different code to scanners, replay old evidence, and substitute public source or artifacts. Build services, compiler/toolchain bootstrap, verifier distribution, OS/browser, attestation hardware/firmware and their revocation mechanisms remain explicit trust assumptions. Runtime memory-corruption attacks and application bugs are not ruled out by launch attestation.

## Backend release materials

Preserve an immutable, publicly downloadable source/materials bundle and detached verification evidence. It should cover the application commit and full source snapshot, exact `Cargo.lock`, Cargo configuration, path dependencies, Git submodules, vendored registry/Git dependencies with source checksums, native dependency sources or pinned packages, generated-code inputs, build tools, features, target architecture, flags, infrastructure definitions and deployment policy. Preserve original sources and licenses rather than relying on a registry remaining online.

Record both the lockfile's full resolution and the actual target/feature dependency graph: the lockfile can contain crates not compiled for this executable. A dependency inventory alone does not establish source completeness. The hermetic build must fail if it needs an undeclared source or network download.

Keep the source archive separate from the runtime image: unused source and dev dependencies need not inflate or execute in production. Record executable hashes as well as the runtime image manifest and filesystem layers, including dynamic libraries, entrypoint, working directory, user and relevant environment. The runtime image is not identical to an enclave measurement; publish a verifiable mapping through the measured runtime or enclave packaging recipe.

### Current offline bundle format

`leptoskit verify record --bundle DIR --out INVENTORY` records a prepared directory with exactly these top-level roles:

- `source/`, including `Cargo.toml` and `Cargo.lock`;
- `dependencies/`, holding dependency sources, and allowed to be empty for a dependency-free backend;
- `infra/`, containing the infrastructure definitions and proposed workload policy;
- `artifacts/`, including the backend executable at `artifacts/backend`;
- `build.json`, the declared build recipe;
- `secrets.json`, a JSON list of unique environment variable names, with no assignments or values.

The recipe requires `repository`, `commit`, `builder_image`, `target`, `package`, `binary`, `features`, `no_default_features`, and `rustflags`. The repository is a GitHub HTTPS URL, the commit is a full lowercase object ID, and the builder image must name an immutable SHA-256 digest. Unknown recipe fields are rejected so unsupported inputs are not silently ignored. This validates the shape of a declaration; it does not contact GitHub, prove the repository is public, match the source to the commit, resolve dependencies, execute the recipe, or establish that all material is present.

The inventory is format version 1, kind `leptoskit-backend-bundle-inventory`, with sorted relative paths, directory entries, file lengths and SHA-256 digests. File Unix permission bits are included where available; permission-bearing inventories cannot silently downgrade to a platform without that metadata. Directory permissions, owners, timestamps, ACLs and extended attributes are not covered. Links and special files are rejected. The root's own metadata is not included. Inputs must be quiescent local files under the verifier's control; this is not an atomic snapshot or protection against a hostile local administrator.

`leptoskit verify check --bundle DIR --manifest INVENTORY --sha256 TRUSTED_DIGEST` first authenticates the exact inventory bytes against the supplied digest, then rejects changed, missing and extra entries, including empty directories. The expected digest must come from independently authenticated release evidence or a previously trusted record. A checksum downloaded alongside an untrusted bundle supplies no authenticity. Both commands explicitly report that they have not verified build provenance or a live deployment. The inventory is unsigned and is not a SLSA attestation.

The caller prepares public input files explicitly. The command does not read process environment values, discover secret files, bundle a working checkout, execute a repository, or upload data. It cannot determine whether a file under `source/` contains a hardcoded secret; publishers must supply public materials only.

## Reproducible backend builder

**Implemented** in `crates/leptoskit_cli/src/verify/build.rs`. `verify build --recipe R --out DIR` writes `DIR/bundle`, `DIR/inventory.json` and an unsigned in-toto statement with a SLSA provenance v1 predicate at `DIR/provenance.json`; `verify reproduce` checks the pinned inventory, compares `source/` and `infra/` with the commit, re-vendors `dependencies/` from the lockfile and compares them, then rebuilds once and compares the executable. What it does, and where it departs from the original sketch below:

- Source is the exact Git tree (`git ls-tree` + `git cat-file --batch`, not `git archive`, so `export-ignore`/`export-subst` cannot change it), with modes 0644/0755 written explicitly. Submodules and symlinks are refused. Git runs with no global/system config, no credential helper and no prompts, so fetching without `--checkout` only works for public repositories and URL rewrites cannot redirect it.
- `cargo vendor --locked --versioned-dirs` is the only networked step; its source mount is read-only. Its printed source replacement is stored as `dependencies/.leptoskit-vendor.toml` (Cargo skips dot-files in a directory source) and passed with `--config`, so the build has no hidden configuration.
- Compilation runs with `--network none`, `--read-only`, `--cap-drop ALL`, `no-new-privileges`, read-only source and dependency mounts, one writable scratch mount, `--pull=never`, and an explicit environment. Nothing from the host environment is passed in. `CARGO_ENCODED_RUSTFLAGS` is always set from the recipe, so repository Cargo configuration rustflags never apply. `RUSTUP_AUTO_INSTALL=0` makes a missing toolchain fail rather than download.
- `SOURCE_DATE_EPOCH` is the constant `315532800` (1980-01-01) rather than the commit time: `reproduce --offline` has no Git history, and the value must be the same everywhere.
- The container platform is the target's native one (`linux/amd64` or `linux/arm64`); emulated cross-architecture builds are not supported.
- Docker builds run as the scratch directory's owner; rootless Podman maps root to the caller.

Tests: unit tests cover the Git export (attributes, modes, submodules, symlinks, prefixes), the exact container arguments, comparisons and provenance determinism. `tests/verification.rs` has an ignored end-to-end test (`--run-ignored only`; needs Docker or Podman and network) that builds a crate whose `build.rs` panics if the network is reachable, reproduces it, and shows that republishing a consistent inventory over changed source, a changed vendored dependency, or a changed executable fails `reproduce`. It passed with Podman 6.1 on arm64 macOS. A manual run built `sharkdp/hexyl` v0.16.0 from GitHub twice to identical bytes and reproduced it from the published inventory.

Not implemented: runtime image builds and their reproducibility, signed provenance (publishers can sign `provenance.json`'s predicate with GitHub artifact attestations; no reusable workflow is provided), and reproduction on a second operator's machine as part of CI.

The original sketch:

Implement one narrow initial profile: a Linux Rust backend targeting `x86_64-unknown-linux-gnu`, with a fixed package, binary, feature list and build flags. Use an immutable builder image digest and fixed platform, and include its Dockerfile/toolchain provenance. A tag such as `rust:latest` cannot identify a build environment. Pin package repositories/snapshots and tool downloads as well as the container's base image.

The preparation phase snapshots the public commit and dependencies without copying untracked files, `.git` credentials or developer environment values. Resolve/vendoring work happens in an isolated preparation container, not by executing repository tooling on the developer's host. Reject unsupported materials such as unaccounted external path dependencies and submodules rather than quietly omitting them.

The compile phase gets only read-only declared inputs and private scratch/output locations. Run `cargo build --release --frozen` with explicit target/package/binary/features/flags. Block container networking as well as Cargo networking: `--frozen` does not sandbox `build.rs`, native build tools or compiler wrappers. Do not mount the Docker socket, SSH agent, host home or shared build cache. Record actual toolchain versions. Define path remapping, locale, time zone and a source-derived `SOURCE_DATE_EPOCH`; remove timestamps/randomness from outputs where necessary.

Build twice in isolated scratch locations, then require matching executable bytes. This detects some nondeterminism but is not independent builder verification. A separate machine/operator must reproduce the same release before the independent-reproduction claim is granted. Test executable and runtime-image reproducibility separately: archive timestamps and signing envelopes can differ even when executable bytes match. Do not weaken matching by stripping behavior-relevant bytes after the comparison fails.

Use standard SLSA/in-toto provenance and Sigstore verification, including pinned issuer/builder/workflow identities, canonical source identity, artifact subjects and build parameters. Do not treat an arbitrary workflow that signs a downloaded artifact as proof of complete source closure. A vetted reusable builder and independent reproduction should make that stronger connection. Keep the inventory detached to avoid embedding a binary's own final hash inside itself.

## Deployment policy, infrastructure and secrets

A release describes artifacts. A separately authorized deployment policy binds a release to approved website/API origins, service identities, supported component versions, workload policies and state/dependency identities. Record both IaC and the generated workload policy after provider/operator transformations. IaC text or a state file alone cannot prove current infrastructure.

Enforce allowed images, mounts, executable loading, entrypoints, configuration, debugging and administrative operations inside the attested boundary. A container's read-only filesystem on an ordinary host is not sufficient: the host can replace containers, modify memory or intercept traffic. Treat the orchestration layer as untrusted where the selected attestation architecture allows it. Avoid claiming a byte-for-byte match for an entire physical cloud or Kubernetes cluster.

Publish secret **names and purposes**, permitted provider/authority identities and references to approved secret versions where appropriate. Keep values private and out of build arguments, public evidence and logs. Non-secret behavior-affecting configuration must be bound to policy too. A name such as `DATABASE_URL` does not authenticate the database, its data or credentials. The secret provisioner must authorize delivery only to an attested workload and deployment identity. Do not publish naive hashes of low-entropy secrets as commitments.

For updates, bind a deployment policy sequence, validity interval and compatible artifact set. Use signed update metadata with expiry and rollback protection, backed by an authenticated current-state view; transparency-log inclusion alone does not establish that a release is current or unrevoked. Support overlap for approved rolling updates without accepting arbitrary old releases. Define key rotation/revocation and recovery before enabling production enforcement.

## Attested execution and the poisoned replica

Use Contrast confidential containers on a supported Linux platform as the proposed reference stack, matching the Kubernetes administrator threat model. The first topology is one protected backend service behind a TCP pass-through load balancer, with no unverified application sidecars or workers. Validate the selected release's threat model, platform support, enforced policy and freshness/relay protections before provisioning it. AWS Nitro is an alternative integration if hosting requirements favor an enclave instead; do not build both adapters in the first milestone. Do not implement hardware evidence parsers or a new transport cryptosystem from scratch.

The runtime must protect the whole application execution path, not just a signer sidecar. Its connection key is generated or securely provisioned inside the boundary, never exported to the host, and authorized for a particular deployment. Fresh evidence binds the key to the approved measurements and policy. The measurement-to-artifact mapping must be checked, not accepted from a self-reported `release_hash` field. Reject debug modes, unapproved runtime/firmware versions and revoked evidence according to the selected stack.

The verifier authenticates the _actual_ application connection before releasing credentials or accepting responses. A separate attestation endpoint followed by unrelated ordinary HTTPS is insufficient. For the first prototype, pass encrypted connections through the load balancer directly to the protected backend. If a gateway terminates the protected channel, include it in the verified boundary and require authenticated downstream peers on every data path. Do not accept a gateway assertion about an ordinary, replaceable upstream.

Every responder must qualify, including workers used to produce user-visible results. One poisoned replica among nine valid replicas must not establish an accepted connection or inject an accepted response. An attacker forwarding an intact encrypted connection is not substituting backend computation, but authorization and machine/deployment binding are still required to prevent accepting another deployment of identical code. Integrations must address their documented relay vulnerabilities.

Attest connection establishment/renewal and authenticate each response with the resulting secure channel. This does not require a hardware attestation on every request. Refuse unverified fallback; test retries, reconnects, WebSockets/streams, caching and rollouts. Authentication failures should give the user a usable error without sending secrets over a weaker path.

## Feasible frontend delivery

### Ordinary browser: evidence discovery

Expose detached release/deployment evidence at a well-known location and make it downloadable by an independent verifier. A website may link to that evidence and the exact source/audits, but its own indicator is informational. A remote scanner and SRI/CSP do not prove what a particular visitor executed when the site itself is malicious. An authenticated backend response alone cannot prevent a malicious frontend from stealing credentials before making that request.

Do not label ordinary browser sessions as verified frontend execution in the first release. A browser extension can provide an independent evidence display, but universal pre-execution enforcement is not assumed. Chromium Manifest V3 limits blocking interception, and Chrome Web Store policy restricts remotely hosted executable code. A generic extension that downloads and executes arbitrary verified app JS/Wasm is not the default plan.

### First enforced client: installed desktop reference host

Start with an independently installed, versioned desktop host and its own protected verification UI. Reuse the existing Wasm guest/runtime and code push bundle machinery. Verify the host's distribution/provenance, the full active guest bundle including lazy chunks, and its allowed runtime plugins. Preserve host-owned trust policy across guest updates; a guest cannot replace the verifier, change accepted roots or draw the authoritative verification status.

All application networking in this profile must pass through the host's attestation-aware transport. Restrict raw networking and plugin escape routes, and authorize plugin capabilities explicitly. No credentials are released until the backend is verified. Pin the intended project and deployment identity independently of guest-supplied metadata. Verify updates before activation; an ordinary publisher signature is necessary but not sufficient to prove public-source provenance or audit coverage.

Use macOS as the first reference implementation, then validate Windows and Linux. Existing iOS/Android hosts are follow-ups with platform-specific signed-host and store-distribution checks. A verified native Wasm client is not a claim that the web frontend bytes are identical.

### Web frontend bytes in an enforced client

If users require the actual web bundle, add a desktop WebView launcher only after the native host prototype. Load a verified static/CSR bundle from a local immutable application origin, verify JS/Wasm/CSS/HTML/assets before execution, and mediate API traffic through the trusted native transport. Restrict remote executable resources, dynamic code generation, service workers, navigation and native bridges. Isolate cookies/storage per project and deployment. Test origin, OAuth/passkey, CORS and cookie compatibility rather than treating a local origin as the production website.

SSR/islands require an additional design: accept dynamic executable HTML only through the attested channel and keep the resource-loading restrictions intact. They are outside the initial static/CSR profile. Never claim arbitrary existing web applications work unchanged in this mode.

## zkTLS, DKIM and zero knowledge

**zkTLS/TLSNotary:** useful later for portable, selectively disclosed evidence that an HTTPS origin returned particular data. It cannot turn a server's claim about its own executable into runtime attestation. A notarized version endpoint can still contain a lie, and one notarized response does not cover another user's session or the whole replica set. TLSNotary introduces a verifier/notary trust model and protocol overhead; these belong in the evidence's claim and validation policy. Do not put zkTLS on the application's mandatory request path for this feature.

**DKIM:** not applicable as a backend execution proof. It establishes responsibility by an email signing domain for signed email content, not which executable generated it. Borrow the separation of signer identity from message content; do not repurpose DKIM email canonicalization or DNS key discovery as the deployment trust anchor.

**Signed HTTP receipts:** if portable request/response evidence is needed, evaluate HTTP Message Signatures (RFC 9421) and Content-Digest (RFC 9530). Define coverage for the complete request/response bodies, method, target, status, deployment/release identity, request nonce, audience and expiry; signatures must come from the attested application boundary. Bind responses to their requests and prevent replay. Signing selected headers or forwarding unverified upstream answers is insufficient. Such receipts provide no confidentiality by themselves and are optional when an authenticated end-to-end channel already meets the requirement.

**Zero knowledge:** not required to keep secret values out of public build/runtime evidence. It may later prove a narrowly defined predicate about private inputs or selectively disclose receipt fields. A proof must bind the witness to the actual authenticated execution/state; proving that some qualifying secret exists does not prove that production loaded it. zkVM proofs of computation could be a separate feature for deterministic business operations, but would also need fresh authenticated inputs/state and do not prove an entire conventional server deployment or replace client verification.

Decision: keep zkTLS, DKIM and a custom ZK system out of the core implementation. Preserve an evidence format that can carry optional, independently verified receipts later.

## Delivery gates

1. **Offline bundle inventory — implemented foundation.** Check pinned inventory before parsing it; reject mutation in every role, absent/extra files and directories, mutable builder identifiers, malformed secret declarations, links and unsupported metadata formats. Require a separately trusted expected digest. Show that replacing both the artifact and inventory fails against the original pin. This is not source completeness or reproducible builds.
2. **Hermetic backend builder — implemented, except runtime images and signed provenance.** Generate materials from a clean public commit, handle the dependency/source closure, produce two equal builds and runtime artifacts, publish standard provenance, and reproduce from another operator's machine. Changing a lockfile, feature, native library or builder must change recorded material or fail comparison. Test undeclared network fetches and absent inputs.
3. **Attested backend prototype.** Pick one stack, authorize protected workload keys and enforce policy. Change one replica, mount a replacement binary, attempt administrative execution, change a behavior-relevant environment value, replay evidence, relay another deployment and route a connection through an unverified gateway. Each prohibited path must fail before user secrets are sent. Also test the accepted direct and rolling-update paths. Document persistent-state rollback/data-integrity boundaries.
4. **Verified desktop guest.** Verify installed host and active guest/source provenance; reject altered entry modules/lazy chunks, downgraded releases, unapproved plugins and changed backend identities. Demonstrate that guest UI cannot grant verifier approval and guest networking cannot bypass the trusted transport.
5. **Optional web launcher and receipts.** Implement the constrained web profile, then expand supported render modes only with concrete compatibility and adversarial evidence. Add portable signed/selectively disclosed receipts only for a real use case.

Never substitute local mocks for the deployment gate: the Docker/Podman engine was unavailable on the development machine when the offline inventory was introduced. That implementation does not claim an end-to-end container rebuild or hardware-attestation test.

## Primary references

- [Solana verified builds](https://solana.com/docs/programs/verified-builds): controlled rebuild and comparison with deployed program bytes.
- [Cargo vendor](https://doc.rust-lang.org/cargo/commands/cargo-vendor.html) and [Cargo build](https://doc.rust-lang.org/cargo/commands/cargo-build.html): source vendoring and the scope of frozen dependency resolution.
- [SLSA artifact verification](https://slsa.dev/spec/v1.2/verifying-artifacts): recognized builders, expected source identity and artifact subjects.
- [GitHub attestations](https://docs.github.com/en/actions/concepts/security/artifact-attestations): authenticated provenance and its limits.
- [AWS Nitro evidence](https://docs.aws.amazon.com/enclaves/latest/user/verify-root.html) and [Contrast attestation](https://docs.edgeless.systems/contrast/architecture/attestation/overview): measurement, policy, key/freshness binding and relay considerations.
- [TLSNotary introduction](https://tlsnotary.org/docs/intro/): TLS data provenance, selective disclosure and notary trust.
- [DKIM, RFC 6376](https://www.rfc-editor.org/rfc/rfc6376.html), [HTTP Message Signatures, RFC 9421](https://www.rfc-editor.org/rfc/rfc9421.html), and [Digest Fields, RFC 9530](https://www.rfc-editor.org/rfc/rfc9530.html).
- [Chrome remotely hosted code policy](https://developer.chrome.com/docs/extensions/develop/migrate/remote-hosted-code) and [webRequest constraints](https://developer.chrome.com/docs/extensions/reference/api/webRequest).
