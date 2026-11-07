//! Schema fidelity diagnostics: what each target can and cannot carry.
#![cfg(feature = "generators")]

use leptoskit_rpc_codegen::ApiFunction;
use leptoskit_rpc_codegen::ApiKind;
use leptoskit_rpc_codegen::ApiParam;
use leptoskit_rpc_codegen::ApiSpec;
use leptoskit_rpc_codegen::Diagnostic;
use leptoskit_rpc_codegen::DiagnosticKind;
use leptoskit_rpc_codegen::GenerateOptions;
use leptoskit_rpc_codegen::HttpInput;
use leptoskit_rpc_codegen::HttpOutput;
use leptoskit_rpc_codegen::TargetLanguage;
use serde_json::json;

/// Shared fixture covering supported shapes, intentional free-form JSON, and
/// real representation loss.
fn fixture() -> ApiSpec {
	ApiSpec::from_json(include_str!("fixtures/schema_fidelity.json"))
		.expect("the fidelity fixture is a valid spec")
}

fn diagnostics(spec: &ApiSpec, language: TargetLanguage) -> Vec<Diagnostic> {
	spec.diagnostics(&GenerateOptions::new(language, "fidelity"))
}

fn at<'a>(diagnostics: &'a [Diagnostic], path: &str) -> Vec<&'a Diagnostic> {
	diagnostics
		.iter()
		.filter(|diagnostic| diagnostic.path == path)
		.collect()
}

fn kinds(diagnostics: &[Diagnostic], path: &str) -> Vec<DiagnosticKind> {
	at(diagnostics, path)
		.iter()
		.map(|diagnostic| diagnostic.kind)
		.collect()
}

#[test]
fn unsupported_constructs_carry_their_schema_path() {
	let diagnostics = diagnostics(&fixture(), TargetLanguage::TypeScript);

	for (path, message, fallback) in [
		("components.Mixed", "more than one", "unknown"),
		("components.Composed", "`allOf` with 2", "unknown"),
		("components.NumberEnum", "non-string", "base type"),
		(
			"functions.mixed_inline.params.value",
			"more than one",
			"unknown",
		),
	] {
		let found = at(&diagnostics, path);
		assert_eq!(found.len(), 1, "{path}: {diagnostics:#?}");
		assert_eq!(found[0].kind, DiagnosticKind::UnsupportedSchema);
		assert!(
			found[0].message.contains(message),
			"{path}: {}",
			found[0].message
		);
		assert!(
			found[0].message.contains(fallback),
			"the message names what TypeScript writes instead: {}",
			found[0].message
		);
	}

	let missing = at(&diagnostics, "functions.missing_ref.output");
	assert_eq!(missing.len(), 1, "{diagnostics:#?}");
	assert_eq!(missing[0].kind, DiagnosticKind::UnresolvedReference);
	assert!(
		missing[0].message.contains("Absent"),
		"{}",
		missing[0].message
	);
	assert!(
		missing[0].message.contains("unknown"),
		"{}",
		missing[0].message
	);
}

#[test]
fn intentional_free_form_json_is_not_a_loss() {
	let diagnostics = diagnostics(&fixture(), TargetLanguage::TypeScript);

	for path in [
		"components.Blob",
		"components.Loose",
		"components.Account.properties.settings",
		"components.Account.properties.labels",
		"components.State",
	] {
		assert!(
			at(&diagnostics, path).is_empty(),
			"{path} is deliberately broad, not lossy: {diagnostics:#?}"
		);
	}

	// A skipped function's schemas are not part of this client at all.
	assert!(
		at(&diagnostics, "components.OnlySkipped").is_empty(),
		"{diagnostics:#?}"
	);
	let skipped = at(&diagnostics, "functions.skipped_stream");
	assert_eq!(skipped.len(), 1);
	assert_eq!(skipped[0].kind, DiagnosticKind::SkippedFunction);
}

#[test]
fn targets_differ_in_what_they_can_represent() {
	let spec = fixture();

	let typescript = diagnostics(&spec, TargetLanguage::TypeScript);
	let python = diagnostics(&spec, TargetLanguage::Python);
	let go = diagnostics(&spec, TargetLanguage::Go);
	let rust = diagnostics(&spec, TargetLanguage::Rust);
	let dart = diagnostics(&spec, TargetLanguage::Dart);
	let cli = diagnostics(&spec, TargetLanguage::Cli);

	for diagnostics in [&typescript, &python] {
		assert!(
			at(diagnostics, "components.Choice").is_empty(),
			"these targets preserve an arbitrary union"
		);
	}
	for diagnostics in [&go, &rust, &dart, &cli] {
		assert_eq!(
			kinds(diagnostics, "components.Choice"),
			[DiagnosticKind::UnionFallback],
			"these targets collapse it"
		);
	}

	// A tagged serde enum is structured in Rust; Go and Dart still flatten
	// every union to their dynamic type.
	assert!(at(&rust, "components.Event").is_empty(), "{rust:#?}");
	for diagnostics in [&go, &dart] {
		assert_eq!(
			kinds(diagnostics, "components.Event"),
			[DiagnosticKind::UnionFallback]
		);
	}

	for diagnostics in [&typescript, &python, &rust] {
		assert!(
			at(diagnostics, "components.Point").is_empty(),
			"these targets carry a fixed tuple"
		);
	}
	for diagnostics in [&go, &dart] {
		assert_eq!(
			kinds(diagnostics, "components.Point"),
			[DiagnosticKind::TupleFallback]
		);
	}

	assert_eq!(
		kinds(&typescript, "components.Account.properties.id"),
		[DiagnosticKind::IntegerPrecision]
	);
	for diagnostics in [&python, &go, &rust, &dart, &cli] {
		assert!(
			at(diagnostics, "components.Account.properties.id").is_empty(),
			"these targets can represent uint64 exactly"
		);
	}

	assert_eq!(
		kinds(&dart, "components.Account.properties.deleted_at"),
		[DiagnosticKind::NullableCollapsed]
	);
	for diagnostics in [&typescript, &python, &go, &rust, &cli] {
		assert!(
			at(diagnostics, "components.Account.properties.deleted_at").is_empty(),
			"these targets keep required-nullable distinct"
		);
	}
}

#[test]
fn openapi_preserves_the_whole_schema() {
	assert!(
		diagnostics(&fixture(), TargetLanguage::OpenApi).is_empty(),
		"the OpenAPI document carries every schema verbatim"
	);
}

/// Every construct these targets model exactly, including optional and
/// nullable fields side by side. Nothing here should be reported.
fn supported_spec() -> ApiSpec {
	let mut spec = ApiSpec::empty("Supported", "1.0.0");
	spec.functions = vec![ApiFunction {
		name: "read".to_owned(),
		kind: ApiKind::Call,
		module_path: "supported::api".to_owned(),
		struct_name: "Read".to_owned(),
		docs: String::new(),
		path: "/api/read".to_owned(),
		method: "POST".to_owned(),
		input: HttpInput::Json,
		output: HttpOutput::Json,
		error_content_type: "text/plain".to_owned(),
		params: vec![
			ApiParam {
				name: "filter".to_owned(),
				docs: String::new(),
				schema: json!({ "$ref": "#/components/schemas/Filter" }),
				required: true,
			},
			ApiParam {
				name: "limit".to_owned(),
				docs: String::new(),
				schema: json!({ "type": "integer", "format": "uint32" }),
				required: false,
			},
		],
		output_schema: json!({ "$ref": "#/components/schemas/User" }),
		internal: false,
		deprecated: false,
		tags: Vec::new(),
		scopes: Vec::new(),
	}];
	spec.schemas.insert(
		"Filter".to_owned(),
		json!({ "type": "string", "enum": ["all", "active"] }),
	);
	spec.schemas.insert(
		"User".to_owned(),
		json!({
			"type": "object",
			"properties": {
				"id": { "type": "integer", "format": "uint32" },
				"name": { "type": "string" },
				"nickname": { "type": "string" },
				"role": { "type": "string", "default": "member" },
				"note": { "type": ["string", "null"] },
				"tags": { "type": "array", "items": { "type": "string" } },
				"counts": {
					"type": "object",
					"additionalProperties": { "type": "integer", "format": "uint32" }
				}
			},
			"required": ["id", "name"]
		}),
	);
	spec
}

#[test]
fn useful_permissive_schemas_are_not_rejected() {
	for target in TargetLanguage::ALL {
		assert!(
			diagnostics(&supported_spec(), *target).is_empty(),
			"{target} should accept ordinary optional and nullable fields"
		);
	}
}
