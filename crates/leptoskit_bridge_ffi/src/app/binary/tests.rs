use std::path::PathBuf;

use leptoskit_native::Color;
use leptoskit_native::FontWeight;
use leptoskit_native::Mutation;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::TextAlign;
use leptoskit_yoga::NodeId;
use leptoskit_yoga::Yoga;
use serde_json::Map;
use serde_json::Value;
use serde_json::json;

use super::encode;

/// Reads a binary transaction back into the JSON transaction's shape, the
/// reference the Swift and Kotlin decoders are held to.
fn decode(bytes: &[u8]) -> Value {
	struct Reader<'a>(&'a [u8]);

	impl Reader<'_> {
		fn take(&mut self, len: usize) -> &[u8] {
			let (head, tail) = self.0.split_at(len);
			self.0 = tail;
			head
		}

		fn u8(&mut self) -> u8 {
			self.take(1)[0]
		}

		fn u32(&mut self) -> u32 {
			u32::from_le_bytes(self.take(4).try_into().unwrap())
		}

		fn f32(&mut self) -> f32 {
			f32::from_le_bytes(self.take(4).try_into().unwrap())
		}

		fn string(&mut self) -> String {
			let len = self.u32() as usize;
			String::from_utf8(self.take(len).to_vec()).unwrap()
		}

		fn props(&mut self) -> Value {
			let present = self.u32();
			let has = |bit: u32| present & (1 << bit) != 0;
			let mut props = Map::new();

			for (bit, key) in [
				(0, "text"),
				(1, "source"),
				(2, "placeholder"),
				(3, "value"),
				(4, "accessibility_label"),
			] {
				if has(bit) {
					props.insert(key.into(), self.string().into());
				}
			}

			for (bit, key) in [(5, "color"), (6, "background"), (7, "border_color")] {
				if has(bit) {
					props.insert(key.into(), self.u32().into());
				}
			}

			for (bit, key) in [(8, "enabled"), (9, "hidden"), (10, "clip")] {
				if has(bit) {
					props.insert(key.into(), (self.u8() != 0).into());
				}
			}

			for (bit, key) in [
				(11, "corner_radius"),
				(12, "border_width"),
				(13, "opacity"),
				(14, "font_size"),
			] {
				if has(bit) {
					props.insert(key.into(), self.f32().into());
				}
			}

			if has(15) {
				let weight = ["regular", "medium", "semibold", "bold"][self.u8() as usize];
				props.insert("font_weight".into(), weight.into());
			}

			if has(16) {
				let align = ["start", "center", "end"][self.u8() as usize];
				props.insert("text_align".into(), align.into());
			}

			if has(31) {
				let Value::Object(rest) = serde_json::from_str(&self.string()).unwrap() else {
					panic!("the rest of the props is an object");
				};

				for (key, value) in rest {
					assert!(props.insert(key, value).is_none(), "a property sent twice");
				}
			}

			Value::Object(props)
		}
	}

	let mut reader = Reader(bytes);
	assert_eq!(reader.take(4), super::MAGIC);
	let root = i64::from_le_bytes(reader.take(8).try_into().unwrap());
	let count = reader.u32();
	let mutations: Vec<Value> = (0..count)
		.map(|_| {
			match reader.u8() {
				1 => {
					let id = reader.u32();
					let len = usize::from(reader.u8());
					let kind = String::from_utf8(reader.take(len).to_vec()).unwrap();
					json!({ "Create": { "id": id, "kind": kind, "props": reader.props() } })
				}
				2 => json!({ "UpdateProps": { "id": reader.u32(), "props": reader.props() } }),
				3 => {
					json!({ "UpdateLayout": {
						"id": reader.u32(), "x": reader.f32(), "y": reader.f32(),
						"width": reader.f32(), "height": reader.f32(),
					} })
				}
				4 => {
					json!({ "InsertChild": {
						"parent": reader.u32(), "child": reader.u32(), "index": reader.u32(),
					} })
				}
				5 => json!({ "RemoveChild": { "parent": reader.u32(), "child": reader.u32() } }),
				6 => json!({ "Delete": { "id": reader.u32() } }),
				op => panic!("unknown mutation {op}"),
			}
		})
		.collect();
	assert!(reader.0.is_empty(), "trailing bytes");

	json!({
		"version": 1,
		"root": if root < 0 { Value::Null } else { root.into() },
		"mutations": mutations,
	})
}

/// Every typed property, properties sent in the JSON form, and every kind of
/// mutation.
fn sample() -> (Option<usize>, Vec<Mutation>) {
	let mut yoga = Yoga::new();
	let ids: Vec<NodeId> = (0..4).map(|_| yoga.create_node()).collect();
	let full = NativeProps {
		text: Some("Card 7 — ✓".into()),
		source: Some("file:///image.png".into()),
		placeholder: Some("Search".into()),
		value: Some("typed".into()),
		accessibility_label: Some("Seventh card".into()),
		color: Some(Color(0x1122_33ff)),
		background: Some(Color(0xffff_ff00)),
		border_color: Some(Color(0x0000_0080)),
		enabled: Some(false),
		hidden: Some(true),
		clip: Some(true),
		corner_radius: Some(8.5),
		border_width: Some(1.0),
		opacity: Some(0.25),
		font_size: Some(13.0),
		font_weight: Some(FontWeight::Semibold),
		text_align: Some(TextAlign::End),
		checked: Some(true),
		id: Some("card-7".into()),
		italic: Some(true),
		..NativeProps::default()
	};
	let mutations = vec![
		Mutation::Create {
			id: ids[0],
			kind: NativeKind::ScrollView,
			props: NativeProps::default(),
		},
		Mutation::Create {
			id: ids[1],
			kind: NativeKind::Text,
			props: full,
		},
		Mutation::Create {
			id: ids[2],
			kind: NativeKind::Custom("map_view"),
			props: NativeProps::text("only text"),
		},
		Mutation::UpdateProps {
			id: ids[2],
			props: NativeProps {
				font_weight: Some(FontWeight::Bold),
				text_align: Some(TextAlign::Center),
				..NativeProps::default()
			},
		},
		Mutation::UpdateLayout {
			id: ids[1],
			x: 16.0,
			y: -2.5,
			width: 358.25,
			height: 17.0,
		},
		Mutation::InsertChild {
			parent: ids[0],
			child: ids[1],
			index: 0,
		},
		Mutation::RemoveChild {
			parent: ids[0],
			child: ids[3],
		},
		Mutation::Delete { id: ids[3] },
	];
	(Some(ids[0].index()), mutations)
}

fn json_transaction(root: Option<usize>, mutations: &[Mutation]) -> Value {
	json!({ "version": 1, "root": root, "mutations": mutations })
}

#[test]
fn the_binary_transaction_carries_exactly_what_the_json_one_does() {
	let (root, mutations) = sample();
	let bytes = encode(root, &mutations, usize::MAX).unwrap();

	assert_eq!(decode(&bytes), json_transaction(root, &mutations));
	let empty = encode(None, &[], usize::MAX).unwrap();
	assert_eq!(empty.len(), 16, "the smallest transaction is its header");
	assert_eq!(decode(&empty), json_transaction(None, &[]));
}

#[test]
fn the_binary_transaction_stops_at_its_byte_limit() {
	let (root, mutations) = sample();
	let bytes = encode(root, &mutations, usize::MAX).unwrap();

	assert_eq!(encode(root, &mutations, bytes.len()).unwrap(), bytes);
	assert!(encode(root, &mutations, bytes.len() - 1).is_err());
}

/// The Kotlin form of the fixture: Android's instrumented tests run on a
/// device, where the repository's files are out of reach.
fn kotlin_fixture(bytes: &[u8], json: &str) -> String {
	use base64::Engine as _;

	format!(
		"@file:Suppress(\"ktlint:standard:max-line-length\")\n\npackage \
		 dev.leptoskit.mobile\n\n// Written by the bridge's `binary` tests; rerun them with\n// \
		 LEPTOSKIT_UPDATE_FIXTURES=1 after changing the format.\n\n/** The sample render \
		 transaction in the binary form, base64 encoded. */\ninternal const val \
		 RENDER_TRANSACTION_BINARY = \"{}\"\n\n/** The same transaction in the JSON form. \
		 */\ninternal const val RENDER_TRANSACTION_JSON = \"\"\"{}\"\"\"\n",
		base64::engine::general_purpose::STANDARD.encode(bytes),
		json
	)
}

/// The shared fixture the Swift and Kotlin decoders read: the sample in both
/// forms. `LEPTOSKIT_UPDATE_FIXTURES=1` rewrites it after a format change.
#[test]
fn the_platform_decoders_fixture_matches_the_encoder() {
	let (root, mutations) = sample();
	let bytes = encode(root, &mutations, usize::MAX).unwrap();
	let json = serde_json::to_vec_pretty(&json_transaction(root, &mutations)).unwrap();
	let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
	let binary_path = directory.join("render-transaction.bin");
	let json_path = directory.join("render-transaction.json");
	let kotlin_path =
		PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("android-tests/RenderBatchFixture.kt");
	let kotlin = kotlin_fixture(
		&bytes,
		&serde_json::to_string(&json_transaction(root, &mutations)).unwrap(),
	);

	if std::env::var_os("LEPTOSKIT_UPDATE_FIXTURES").is_some() {
		std::fs::create_dir_all(&directory).unwrap();
		std::fs::write(&binary_path, &bytes).unwrap();
		std::fs::write(&json_path, &json).unwrap();
		std::fs::write(&kotlin_path, &kotlin).unwrap();
	}

	assert_eq!(
		std::fs::read_to_string(&kotlin_path).unwrap(),
		kotlin,
		"rerun with LEPTOSKIT_UPDATE_FIXTURES=1 after changing the format"
	);

	assert_eq!(
		std::fs::read(&binary_path).unwrap(),
		bytes,
		"rerun with LEPTOSKIT_UPDATE_FIXTURES=1 after changing the format"
	);
	assert_eq!(
		serde_json::from_slice::<Value>(&std::fs::read(&json_path).unwrap()).unwrap(),
		json_transaction(root, &mutations)
	);
}
