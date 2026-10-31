# OpenID certification for `leptoskit_idp`

What it takes to certify `leptoskit_idp` as an OpenID Provider, what blocks it today, and the order to do it in. The protocol notes are in [auth-plan.md](auth-plan.md#leptoskit_idp-sign-in-with-your-app); this file is only about certification.

## How certification works

OpenID certification is self-certification. The implementer runs the OpenID Foundation's conformance suite against a deployment, gets a passing result for each test plan of a profile, signs the Foundation's certification statement, pays its fee, and submits the plan results; the Foundation then lists the implementation. The suite runs either on the Foundation's hosted instance, which needs the provider reachable from the internet, or self-hosted from its source (Java, MongoDB, Docker Compose). Check the Foundation's certification pages for the current statement, fees, and submission address before starting: they change, so none is quoted here.

## Which profiles fit

| Profile                                            | Test plan                                           | Fits?                                                                                               |
| -------------------------------------------------- | --------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| Basic OP                                           | `oidcc-basic-certification-test-plan`               | Blocked by PKCE (below); otherwise the code flow, `userinfo`, `prompt`, `max_age`, `claims` are in. |
| Config OP                                          | `oidcc-config-certification-test-plan`              | Blocked by PKCE; discovery and JWKS are in.                                                         |
| Dynamic OP                                         | `oidcc-dynamic-certification-test-plan`             | Blocked by PKCE; dynamic registration is in since IdP 3.                                            |
| RP-Initiated Logout OP                             | `oidcc-rp-initiated-logout-certification-test-plan` | Likely: `end_session` with `id_token_hint` and registered `post_logout_redirect_uri`s.              |
| Implicit OP, Hybrid OP                             |                                                     | No: neither flow exists, deliberately.                                                              |
| Form Post OP                                       |                                                     | No: `response_mode=form_post` isn't supported.                                                      |
| Session Management, Front- and Back-Channel Logout |                                                     | No: none is built.                                                                                  |
| FAPI 2.0 Security Profile                          | the suite's FAPI 2.0 Security Profile plan          | Closest fit for PAR + PKCE + DPoP + `private_key_jwt` + ES256; needs the gaps below checked first.  |

The `oidcc-*` names are the suite's plan identifiers; confirm them, and find the FAPI 2.0 plan's, in the suite version used.

## What blocks it

1. **PKCE is always required.** The Basic, Config, and Dynamic plans send authorization requests without `code_challenge`, and `/oauth/authorize` refuses those for every client. RFC 9700 §2.1.1 allows a confidential client that sends `nonce` to skip PKCE, so a per-client opt-in would unblock the plans. This is the owner's decision and stays as it is: PKCE is required. Until that changes, those three profiles can't pass, and only RP-Initiated Logout and FAPI 2.0 (which require PKCE) are worth running.
2. **A purpose-built test server.** The suite drives sign-in and consent with a scripted browser through configured page selectors, and email codes can't be scripted. `examples/idp_conformance` (not built yet) needs:
   - an `https` issuer that the suite's containers and browser both reach, with an RSA key (`rsa_key_env`), since the OpenID plans expect RS256 ID tokens;
   - an HTML sign-in page for one seeded user (a password-less test-only form behind a build flag, never in a release build), and an HTML consent page that approves on submit;
   - `[auth.idp]` with `dynamic_registration = true` and no `registration_token_env` for the Dynamic plan, and for the static plans two pre-registered clients whose redirect URI is the suite's callback for the plan's alias (`client_secret_basic` and `private_key_jwt` variants; `client_secret_jwt` isn't supported, so don't pick that variant);
   - a scope set with `openid`, `profile`, and `email`, and `Claims` that release `name`, `email`, and `email_verified`.
3. **A container runtime in CI.** The nightly job builds the suite, starts MongoDB and the suite with Docker Compose, starts the test server, and runs each plan with the suite's `scripts/run-test-plan.py`, failing on any failed module.

## What the provider doesn't do, which some modules try

Expect these modules to be skipped or to warn, and check each against the plan's pass criteria when it first runs: request objects (`request`, `request_uri`; discovery says both are unsupported), `display`, `ui_locales`, `claims_locales`, `acr_values` and the `acr` claim, `prompt=select_account`, `client_secret_jwt`, and `jwks_uri` for clients (dynamic registration takes inline `jwks` only).

For FAPI 2.0, check first: DPoP nonces (`DPoP-Nonce`) and binding the code to the key at the authorization endpoint (`dpop_jkt`) aren't built; `require_pushed_authorization_requests` can't be set per client; mutual-TLS client authentication and sender-constraining don't exist, so pick the `private_key_jwt` and DPoP variants.

## The order to do it in

1. Decide the PKCE question, or accept that only RP-Initiated Logout and FAPI 2.0 are in reach.
2. Build `examples/idp_conformance` as above, with an end-to-end test that its HTML sign-in and consent forms work without a browser.
3. Add the `workflow_dispatch` and nightly conformance job, self-hosting the suite, and run every plan above that fits. Fix what fails.
4. Deploy the test server at a public `https` address, run the same plans on the Foundation's hosted suite, and export the results.
5. Sign the certification statement, pay the fee, and submit the results.
6. Keep the nightly job as the release gate for `leptoskit_idp`, as the plan says.
