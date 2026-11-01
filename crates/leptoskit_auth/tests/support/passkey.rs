//! A software passkey for the tests: discoverable P-256 credentials made at
//! test time, which verify their user and count their signatures, as a
//! platform authenticator does. It speaks WebAuthn's JSON, so the server
//! checks real attestation and assertion bytes.

#![allow(dead_code, reason = "each test file uses part of this")]

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use leptoskit_auth::CreationChallengeResponse;
use leptoskit_auth::PublicKeyCredential;
use leptoskit_auth::RegisterPublicKeyCredential;
use leptoskit_auth::RequestChallengeResponse;
use ring::digest::SHA256;
use ring::digest::digest;
use ring::rand::SystemRandom;
use ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING;
use ring::signature::EcdsaKeyPair;
use ring::signature::KeyPair;
use serde_json::json;

/// A device's passkeys, for one origin.
pub struct SoftPasskey {
	origin: String,
	credentials: Vec<Credential>,
	/// Whether new passkeys are synced, like a password manager's.
	pub synced: bool,
}

struct Credential {
	id: Vec<u8>,
	key: EcdsaKeyPair,
	user: Vec<u8>,
	rp_id: String,
	counter: u32,
}

const UP: u8 = 0x01;
const UV: u8 = 0x04;
const BE: u8 = 0x08;
const BS: u8 = 0x10;
const AT: u8 = 0x40;

impl SoftPasskey {
	/// A device with no passkeys, on pages from `origin`.
	pub fn new(origin: &str) -> Self {
		Self {
			origin: origin.to_owned(),
			credentials: Vec::new(),
			synced: false,
		}
	}

	/// How many passkeys it holds.
	pub fn len(&self) -> usize {
		self.credentials.len()
	}

	/// Makes a passkey for `options`, as `navigator.credentials.create`.
	pub fn register(&mut self, options: &CreationChallengeResponse) -> RegisterPublicKeyCredential {
		let options = serde_json::to_value(options).unwrap();
		let public_key = &options["publicKey"];
		let rp_id = public_key["rp"]["id"].as_str().unwrap().to_owned();
		let user = URL_SAFE_NO_PAD
			.decode(public_key["user"]["id"].as_str().unwrap())
			.unwrap();
		let challenge = public_key["challenge"].as_str().unwrap();
		let rng = SystemRandom::new();
		let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
		let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
			.unwrap();

		let mut id = vec![0; 32];
		ring::rand::SecureRandom::fill(&rng, &mut id).unwrap();
		let point = key.public_key().as_ref();
		let (x, y) = point[1..].split_at(32);

		let mut cose = vec![0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20];
		cose.extend_from_slice(x);
		cose.extend_from_slice(&[0x22, 0x58, 0x20]);
		cose.extend_from_slice(y);

		let mut auth_data = digest(&SHA256, rp_id.as_bytes()).as_ref().to_vec();
		auth_data.push(UP | UV | AT | self.backup_flags());
		auth_data.extend_from_slice(&0_u32.to_be_bytes());
		auth_data.extend_from_slice(&[0; 16]);
		auth_data.extend_from_slice(&u16::try_from(id.len()).unwrap().to_be_bytes());
		auth_data.extend_from_slice(&id);
		auth_data.extend_from_slice(&cose);

		let mut attestation = vec![0xa3];
		attestation.extend(text("fmt"));
		attestation.extend(text("none"));
		attestation.extend(text("attStmt"));
		attestation.push(0xa0);
		attestation.extend(text("authData"));
		attestation.extend(bytes(&auth_data));

		let client_data = json!({
			"type": "webauthn.create",
			"challenge": challenge,
			"origin": self.origin,
			"crossOrigin": false,
		})
		.to_string();
		let encoded_id = URL_SAFE_NO_PAD.encode(&id);
		self.credentials.push(Credential {
			id,
			key,
			user,
			rp_id,
			counter: 0,
		});

		serde_json::from_value(json!({
			"id": encoded_id,
			"rawId": encoded_id,
			"type": "public-key",
			"response": {
				"attestationObject": URL_SAFE_NO_PAD.encode(attestation),
				"clientDataJSON": URL_SAFE_NO_PAD.encode(client_data),
			},
			"extensions": {},
		}))
		.unwrap()
	}

	/// Signs `options` with the first passkey they allow, or, when they
	/// allow any, the newest one for the site, as
	/// `navigator.credentials.get`.
	pub fn sign(&mut self, options: &RequestChallengeResponse) -> PublicKeyCredential {
		let index = self.pick(options).expect("a passkey the options allow");
		self.sign_with(index, options)
	}

	/// Which passkey the device would offer for `options`.
	pub fn pick(&self, options: &RequestChallengeResponse) -> Option<usize> {
		let options = serde_json::to_value(options).unwrap();
		let public_key = &options["publicKey"];
		let rp_id = public_key["rpId"].as_str().unwrap();
		let allowed: Vec<Vec<u8>> = public_key["allowCredentials"]
			.as_array()
			.map(|list| {
				list.iter()
					.map(|credential| {
						URL_SAFE_NO_PAD
							.decode(credential["id"].as_str().unwrap())
							.unwrap()
					})
					.collect()
			})
			.unwrap_or_default();
		self.credentials
			.iter()
			.enumerate()
			.rev()
			.find(|(_, credential)| {
				credential.rp_id == rp_id
					&& (allowed.is_empty() || allowed.contains(&credential.id))
			})
			.map(|(index, _)| index)
	}

	/// Signs `options` with the passkey at `index`, whether or not they
	/// allow it.
	pub fn sign_with(
		&mut self,
		index: usize,
		options: &RequestChallengeResponse,
	) -> PublicKeyCredential {
		let options = serde_json::to_value(options).unwrap();
		let challenge = options["publicKey"]["challenge"]
			.as_str()
			.unwrap()
			.to_owned();
		let backup = self.backup_flags();
		let origin = self.origin.clone();
		let credential = &mut self.credentials[index];
		credential.counter += 1;
		let mut auth_data = digest(&SHA256, credential.rp_id.as_bytes())
			.as_ref()
			.to_vec();
		auth_data.push(UP | UV | backup);
		auth_data.extend_from_slice(&credential.counter.to_be_bytes());
		let client_data = json!({
			"type": "webauthn.get",
			"challenge": challenge,
			"origin": origin,
			"crossOrigin": false,
		})
		.to_string();
		let mut signed = auth_data.clone();
		signed.extend_from_slice(digest(&SHA256, client_data.as_bytes()).as_ref());
		let signature = credential.key.sign(&SystemRandom::new(), &signed).unwrap();
		let id = URL_SAFE_NO_PAD.encode(&credential.id);

		serde_json::from_value(json!({
			"id": id,
			"rawId": id,
			"type": "public-key",
			"response": {
				"authenticatorData": URL_SAFE_NO_PAD.encode(auth_data),
				"clientDataJSON": URL_SAFE_NO_PAD.encode(client_data),
				"signature": URL_SAFE_NO_PAD.encode(signature.as_ref()),
				"userHandle": URL_SAFE_NO_PAD.encode(&credential.user),
			},
			"extensions": {},
		}))
		.unwrap()
	}

	/// Sets the passkey at `index`'s signature counter, as a cloned key
	/// that fell behind would have it.
	pub fn set_counter(&mut self, index: usize, counter: u32) {
		self.credentials[index].counter = counter;
	}

	/// The same device, on pages from another origin, such as a phishing
	/// site's.
	pub fn on(mut self, origin: &str) -> Self {
		origin.clone_into(&mut self.origin);
		self
	}

	fn backup_flags(&self) -> u8 {
		if self.synced { BE | BS } else { 0 }
	}
}

/// A CBOR text string.
fn text(value: &str) -> Vec<u8> {
	let mut out = vec![0x60 | u8::try_from(value.len()).unwrap()];
	out.extend_from_slice(value.as_bytes());
	out
}

/// A CBOR byte string.
fn bytes(value: &[u8]) -> Vec<u8> {
	let mut out = vec![0x59];
	out.extend_from_slice(&u16::try_from(value.len()).unwrap().to_be_bytes());
	out.extend_from_slice(value);
	out
}
