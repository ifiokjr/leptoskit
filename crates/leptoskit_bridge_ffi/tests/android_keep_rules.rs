//! Release builds minify the app's Kotlin with R8, which renames any member
//! no rule keeps. A member the JNI library looks up by name and the keep rules
//! miss compiles, passes debug builds, and crashes the release app at launch.

const SOURCES: [&str; 2] = [
	include_str!("../android/bridge.cpp"),
	include_str!("../mobile/app.cpp"),
];
const RULES: &str = include_str!("../android/proguard-rules.pro");

/// The string literals passed to `call(`, in source order.
fn literals<'a>(source: &'a str, call: &str) -> Vec<Vec<&'a str>> {
	source
		.match_indices(call)
		.map(|(start, _)| {
			let arguments = &source[start + call.len()..];
			let mut literals = Vec::new();
			let mut open = None;

			// Signatures such as "(J)V" hold parentheses, so the call ends at
			// the first one outside a literal.
			for (index, character) in arguments.char_indices() {
				match (character, open) {
					('"', None) => open = Some(index + 1),
					('"', Some(from)) => {
						literals.push(&arguments[from..index]);
						open = None;
					}

					(')', None) => return literals,
					_ => {}
				}
			}

			panic!("unterminated {call}");
		})
		.collect()
}

#[test]
fn every_member_jni_names_is_kept() {
	for source in SOURCES {
		for (call, suffix) in [
			("GetMethodID(", "("),
			("GetStaticMethodID(", "("),
			("GetFieldID(", ";"),
			("GetStaticFieldID(", ";"),
		] {
			for literals in literals(source, call) {
				let [name, _signature] = literals[..] else {
					panic!("{call} needs a literal name and signature: {literals:?}");
				};

				// Constructors belong to classes found by name, checked below.
				if name == "<init>" {
					continue;
				}

				assert!(
					RULES.contains(&format!(" {name}{suffix}")),
					"android/proguard-rules.pro does not keep `{name}`, which JNI looks up by name"
				);
			}
		}
	}
}

#[test]
fn every_leptoskit_class_jni_finds_is_kept() {
	for source in SOURCES {
		for literals in literals(source, "FindClass(") {
			let [class] = literals[..] else {
				panic!("FindClass needs a literal class name: {literals:?}");
			};

			if class.starts_with("dev/leptoskit/") {
				assert!(
					class.starts_with("dev/leptoskit/bridge/")
						&& RULES.contains("-keep class dev.leptoskit.bridge.** { *; }"),
					"android/proguard-rules.pro does not keep `{class}`, which JNI finds by name"
				);
			}
		}
	}
}
