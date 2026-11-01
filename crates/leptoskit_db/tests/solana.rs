//! Property tests for the Solana scalars: base58 round trips, and parsing
//! that accepts exactly the spellings of the right number of bytes.

#![cfg(all(feature = "solana", feature = "regex"))]

use leptoskit_db::Pubkey;
use leptoskit_db::Signature;
use leptoskit_db::base58;
use proptest::prelude::*;

proptest! {
	#[test]
	fn base58_round_trips(bytes in prop::collection::vec(any::<u8>(), 0..96)) {
		let text = base58::encode(&bytes);
		prop_assert_eq!(base58::decode(&text), Some(bytes));
	}

	#[test]
	fn every_spelling_is_the_only_one(text in "[1-9A-HJ-NP-Za-km-z]{0,60}") {
		let bytes = base58::decode(&text).expect("the alphabet decodes");
		prop_assert_eq!(base58::encode(&bytes), text);
	}

	#[test]
	fn addresses_round_trip(bytes in any::<[u8; 32]>()) {
		let address = Pubkey::from_bytes(bytes);
		prop_assert_eq!(address.to_bytes(), bytes);
		prop_assert_eq!(Pubkey::parse(address.as_inner().clone()).unwrap(), address);
	}

	#[test]
	fn signatures_round_trip(bytes in prop::collection::vec(any::<u8>(), 64)) {
		let bytes: [u8; 64] = bytes.try_into().unwrap();
		let signature = Signature::from_bytes(bytes);
		prop_assert_eq!(signature.to_bytes(), bytes);
	}

	#[test]
	fn only_32_bytes_parse_as_an_address(bytes in prop::collection::vec(any::<u8>(), 0..64)) {
		let text = base58::encode(&bytes);
		prop_assert_eq!(Pubkey::parse(text).is_ok(), bytes.len() == 32);
	}

	#[test]
	fn arbitrary_text_never_panics(text in "\\PC{0,100}") {
		let parsed = Pubkey::parse(text.clone());

		if let Ok(address) = parsed {
			prop_assert_eq!(Pubkey::from_bytes(address.to_bytes()), address);
		}

		let _ = Signature::parse(text.clone());
		let _ = base58::decode_exact::<32>(&text);
	}
}
