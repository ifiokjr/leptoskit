//! A software Solana wallet for tests: an ed25519 key made at test time,
//! which signs Sign-In With Solana messages as a browser wallet would.

#![allow(dead_code, reason = "each test uses part of it")]

use ed25519_dalek::Signer;
use ed25519_dalek::SigningKey;
use leptoskit_auth::SiwsInput;
use leptoskit_auth::WalletProof;
use leptoskit_db::Pubkey;
use leptoskit_db::Signature;
use ring::rand::SecureRandom;

pub struct SoftwareWallet {
	key: SigningKey,
}

impl SoftwareWallet {
	pub fn new() -> Self {
		let mut seed = [0; 32];
		ring::rand::SystemRandom::new().fill(&mut seed).unwrap();
		Self {
			key: SigningKey::from_bytes(&seed),
		}
	}

	pub fn address(&self) -> Pubkey {
		Pubkey::from_bytes(self.key.verifying_key().to_bytes())
	}

	/// Signs `text` as it is.
	pub fn sign_text(&self, text: &str) -> WalletProof {
		WalletProof {
			address: self.address(),
			message: text.to_owned(),
			signature: Signature::from_bytes(self.key.sign(text.as_bytes()).to_bytes()),
		}
	}

	/// What `solana:signIn` returns for `input`.
	pub fn sign_in(&self, input: &SiwsInput) -> WalletProof {
		self.sign_text(&input.message(&self.address()))
	}
}
