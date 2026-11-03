let original;
let requests;
let secondFactors;
let fixtures;
let walletBehavior;
let walletListener;
let credentialMethods;
let conditionalMethod;
let passkeyBehavior;

export function install(initiallySignedIn) {
	let signedIn = initiallySignedIn;
	secondFactors = ["totp", "recovery_code", "passkey"];
	original = globalThis.fetch;
	requests = [];
	fixtures = { walletsEnabled: false, linked: [], passkeys: [] };
	walletBehavior = "ok";
	const session = {
		id: "00000000-0000-4000-8000-000000000001",
		account_id: "00000000-0000-4000-8000-000000000002",
		aal: "aal1",
		authenticated_at: "2026-11-01T12:00:00Z",
		mfa_at: null,
		last_seen_at: "2026-11-01T12:00:00Z",
		idle_expires_at: "2026-11-02T12:00:00Z",
		expires_at: "2026-11-03T12:00:00Z",
		device: "Browser",
		revoked_at: null,
		created_at: "2026-11-01T12:00:00Z",
	};
	const mfa = { totp: false, recovery_codes: 0, passkeys: 0, required: false };
	let stepped = false;
	globalThis.fetch = async (input, init) => {
		const request = new Request(input, init);
		const path = new URL(request.url).pathname;
		const body = request.method === "POST" ? await request.text() : "";
		const parsed = body ? JSON.parse(body) : null;
		requests.push({ path, method: request.method, body: parsed });
		let value = {};
		let status = 200;
		switch (`${request.method} ${path}`) {
			case "GET /auth/session":
				value = { session: signedIn ? session : null };
				break;
			case "POST /auth/sign-out":
				signedIn = false;
				break;
			case "POST /auth/email/send":
				break;
			case "POST /auth/email/verify":
				if (parsed.code === "DIRECT") {
					signedIn = true;
					value = { session, signed_up: false };
				} else if (parsed.code === "ABCD-EFGH") {
					value = {
						second_factor: { factors: secondFactors, expires_in: 300 },
					};
				} else {
					status = 400;
					value = { error: "invalid_code", message: "that code was refused" };
				}
				break;
			case "POST /auth/second-factor":
				if (parsed.code === "654321") {
					signedIn = true;
					value = { session, signed_up: false };
				} else {
					status = 400;
					value = { error: "sign_in_expired", message: "sign-in expired" };
				}
				break;
			case "GET /auth/mfa":
				value = mfa;
				break;
			case "GET /oauth/grants":
			case "GET /auth/sessions":
			case "GET /auth/providers":
			case "GET /auth/identities":
				value = [];
				break;
			case "GET /auth/passkeys":
				value = fixtures.passkeys;
				break;
			case "GET /auth/wallets":
				value = fixtures.linked;
				break;
			case "GET /auth/merge":
				value = null;
				break;
			case "GET /auth/wallets/config":
				value = {
					enabled: fixtures.walletsEnabled,
					sign_in: fixtures.walletsEnabled,
					chain_id: "mainnet",
				};
				break;
			case "POST /auth/wallets/link/options":
			case "POST /auth/wallets/proof/options":
			case "POST /auth/wallets/sign-in/options":
				value = {
					input: {
						domain: "fixture.example",
						uri: "https://fixture.example",
						version: "1",
						nonce: "fixture-nonce",
					},
				};
				if (parsed?.address) value.input.address = parsed.address;
				break;
			case "POST /auth/wallets/link": {
				const existing = fixtures.linked.length > 0;
				const wallet = {
					id: "00000000-0000-4000-8000-000000000031",
					chain: "solana",
					address: parsed.proof.address,
					label: "Fixture account",
					can_sign_in: false,
					is_public: false,
					verified_at: session.created_at,
					last_proved_at: session.created_at,
				};
				if (!existing) fixtures.linked.push(wallet);
				value = {
					outcome: existing ? "already_linked" : "linked",
					wallet: fixtures.linked[0],
				};
				break;
			}
			case "POST /auth/wallets/proof":
				value = fixtures.linked[0];
				break;
			case "POST /auth/wallets/update":
				Object.assign(
					fixtures.linked[0],
					Object.fromEntries(
						Object.entries(parsed).filter(([, value]) => value !== null),
					),
				);
				value = fixtures.linked[0];
				break;
			case "POST /auth/wallets/unlink":
				fixtures.linked = [];
				break;
			case "POST /auth/wallets/sign-in":
				signedIn = true;
				value = { session, signed_up: false };
				break;
			case "POST /auth/totp":
				value = {
					secret: "JBSWY3DPEHPK3PXP",
					uri:
						"otpauth://totp/Example:ada?secret=JBSWY3DPEHPK3PXP&issuer=Example",
				};
				break;
			case "POST /auth/totp/confirm":
				mfa.totp = true;
				mfa.recovery_codes = 2;
				value = {
					session,
					recovery_codes: { codes: ["FIRST-RECOVERY", "SECOND-RECOVERY"] },
				};
				break;
			case "POST /auth/recovery-codes":
				value = { codes: ["NEW-RECOVERY", "NEW-SECOND"] };
				break;
			case "POST /auth/totp/disable":
				if (!stepped) {
					status = 403;
					value = { error: "step_up_required", message: "Confirm it is you" };
				} else {
					mfa.totp = false;
					mfa.recovery_codes = 0;
				}
				break;
			case "POST /auth/step-up":
				if (parsed.code === "654321") {
					stepped = true;
					value = { session, signed_up: false };
				} else {
					status = 403;
					value = { error: "invalid_code", message: "that code was refused" };
				}
				break;
			case "POST /auth/passkey/register/options":
				value = {
					publicKey: {
						rp: { id: "localhost", name: "Fixture" },
						user: { id: "AAAA", name: "ada", displayName: "Ada" },
						challenge: "AAAA",
						pubKeyCredParams: [],
					},
				};
				break;
			case "POST /auth/passkey/register": {
				const passkey = {
					id: "00000000-0000-4000-8000-000000000041",
					name: parsed.name || "Browser passkey",
					backed_up: true,
					last_used_at: null,
					created_at: session.created_at,
				};
				fixtures.passkeys.push(passkey);
				mfa.passkeys = fixtures.passkeys.length;
				value = { passkey, session };
				break;
			}
			case "POST /auth/passkeys/remove":
				fixtures.passkeys = [];
				mfa.passkeys = 0;
				break;
			case "POST /auth/passkey/sign-in/options":
			case "POST /auth/passkey/second-factor/options":
			case "POST /auth/passkey/step-up/options":
				if (!credentialMethods) {
					status = 400;
					value = { error: "unsupported", message: "passkeys unavailable" };
				} else {value = {
						options: {
							publicKey: {
								challenge: "AAAA",
								rpId: "localhost",
								allowCredentials: [],
								userVerification: "required",
							},
							mediation: "conditional",
						},
						token: null,
					};}
				break;
			case "POST /auth/passkey/sign-in":
			case "POST /auth/passkey/second-factor":
				signedIn = true;
				value = { session, signed_up: false };
				break;
			case "POST /auth/passkey/step-up":
				stepped = true;
				value = { session, signed_up: false };
				break;
			default:
				throw new Error(`Unexpected auth request: ${request.method} ${path}`);
		}
		return new Response(JSON.stringify(value), {
			status,
			headers: { "content-type": "application/json" },
		});
	};
}

export function calls() {
	return JSON.stringify(requests);
}
export function restore() {
	globalThis.fetch = original;
	if (walletListener) {
		window.removeEventListener("wallet-standard:app-ready", walletListener);
	}
	walletListener = undefined;
	if (credentialMethods) {
		for (const method of ["get", "create"]) {
			const descriptor = credentialMethods[method];
			if (descriptor) {
				Object.defineProperty(navigator.credentials, method, descriptor);
			} else delete navigator.credentials[method];
		}
		Object.defineProperty(
			PublicKeyCredential,
			"isConditionalMediationAvailable",
			conditionalMethod,
		);
		credentialMethods = undefined;
	}
}

export function factors(json) {
	secondFactors = JSON.parse(json);
}

// Software wallets exercise the browser boundary; the auth server's separate
// tests verify real signatures. Neither fixture asks an installed wallet to sign.
export function enable_wallets() {
	fixtures.walletsEnabled = true;
	const account = {
		address: "11111111111111111111111111111111",
		chains: ["solana:mainnet"],
	};
	const sign = (message) => {
		if (walletBehavior === "cancel") {
			throw { code: 4001, message: "user rejected" };
		}
		return {
			account,
			signedMessage: walletBehavior === "prefix"
				? new TextEncoder().encode("prefixed")
				: message,
			signature: new Uint8Array(walletBehavior === "short" ? 63 : 64),
		};
	};
	const text = {
		name: "Fixture Text",
		icon: "",
		chains: ["solana:mainnet"],
		features: {
			"standard:connect": {
				connect: async () => ({
					accounts: walletBehavior === "no_account" ? [] : [account],
				}),
			},
			"solana:signMessage": {
				signMessage: async (input) => [sign(input.message)],
			},
		},
	};
	const siws = {
		name: "Fixture SIWS",
		icon: "",
		chains: ["solana:mainnet"],
		features: {
			"solana:signIn": {
				signIn:
					async () => [sign(new TextEncoder().encode("fixture SIWS message"))],
			},
		},
	};
	const wrong = {
		name: "Other chain",
		chains: ["ethereum:1"],
		features: siws.features,
	};
	const register = (api) => api.register(text, siws, siws, wrong);
	walletListener = (event) => register(event.detail);
	window.addEventListener("wallet-standard:app-ready", walletListener);
	window.dispatchEvent(
		new CustomEvent("wallet-standard:register-wallet", { detail: register }),
	);
}

export function wallet_mode(mode) {
	walletBehavior = mode;
}

// Software credentials exercise conversion and cancellation without opening a
// platform passkey prompt. Server tests separately verify WebAuthn signatures.
export function enable_passkeys() {
	passkeyBehavior = "ok";
	credentialMethods = Object.fromEntries(
		["get", "create"].map(
			(method) => [
				method,
				Object.getOwnPropertyDescriptor(navigator.credentials, method),
			],
		),
	);
	conditionalMethod = Object.getOwnPropertyDescriptor(
		PublicKeyCredential,
		"isConditionalMediationAvailable",
	);
	Object.defineProperty(
		PublicKeyCredential,
		"isConditionalMediationAvailable",
		{ configurable: true, value: async () => false },
	);
	for (const method of ["get", "create"]) {
		Object.defineProperty(navigator.credentials, method, {
			configurable: true,
			value: async (options) => {
				if (passkeyBehavior !== "ok") {
					throw new DOMException("fixture credential refusal", passkeyBehavior);
				}
				if (options.signal.aborted) {
					throw new DOMException("aborted", "AbortError");
				}
				const credential = Object.create(PublicKeyCredential.prototype);
				Object.defineProperties(credential, {
					rawId: { value: new Uint8Array([1, 2, 3]).buffer },
					response: {
						value: {
							clientDataJSON: new Uint8Array([4]).buffer,
							attestationObject: new Uint8Array([5]).buffer,
							authenticatorData: new Uint8Array([6]).buffer,
							signature: new Uint8Array([7]).buffer,
						},
					},
					getClientExtensionResults: { value: () => ({}) },
				});
				return credential;
			},
		});
	}
}

export function passkey_mode(mode) {
	passkeyBehavior = mode;
}
