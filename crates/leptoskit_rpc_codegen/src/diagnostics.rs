//! Schema fidelity diagnostics for the generated clients.
//!
//! The generators trade type detail for broad compatibility in a few places:
//! a union that is not a serde enum becomes `any`, a fixed tuple becomes a
//! list of `any`, an unresolvable `$ref` becomes `unknown`. That is fine by
//! default, but silently losing the shape makes a generated SDK look more
//! precise than it is. This module walks the same schemas the generators see
//! and reports every place where a target cannot carry the schema's shape.
//!
//! Deliberately unconstrained JSON — `true`, `{}`, `additionalProperties:
//! true`, an array without `items` — is represented as `unknown`/`Any`/`any`
//! by design and is not reported. Only dropped structure is.

use std::fmt;

use serde_json::Value;

use crate::generate::GenerateOptions;
use crate::generate::TargetLanguage;
use crate::generate::skip_reason;
use crate::schema::Shape;
use crate::schema::analyze;
use crate::schema::has_default;
use crate::schema::recognize_enum;
use crate::spec::ApiSpec;

/// What kind of detail a target cannot carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagnosticKind {
	/// A schema construct the analyzer does not model.
	UnsupportedSchema,
	/// A `$ref` that names no schema in the specification.
	UnresolvedReference,
	/// A union the target collapses to its dynamic type.
	UnionFallback,
	/// A fixed-length tuple the target collapses to an untyped list.
	TupleFallback,
	/// Integer values a target's number type cannot represent exactly.
	IntegerPrecision,
	/// A target that cannot distinguish an absent value from `null`.
	NullableCollapsed,
	/// A function left out because the target cannot speak its encoding.
	SkippedFunction,
}

/// One place where a target cannot faithfully generate the published RPC,
/// because a schema does not fit its types or a function's encoding cannot be
/// spoken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
	/// The target that loses the detail.
	pub target: TargetLanguage,
	/// Where in the specification, such as
	/// `components.Todo.properties.id`.
	pub path: String,
	/// What was lost.
	pub kind: DiagnosticKind,
	/// What the target generates instead, and why it matters.
	pub message: String,
}

impl fmt::Display for Diagnostic {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			formatter,
			"{}: {}: {}",
			self.target, self.path, self.message
		)
	}
}

/// Reports every schema detail `options.language` cannot carry.
///
/// The specification is filtered exactly as [`ApiSpec::generate`] filters it,
/// so an excluded or internal function does not produce warnings, and a
/// schema only a skipped function referenced is not reported.
#[must_use]
pub(crate) fn diagnose(spec: &ApiSpec, options: &GenerateOptions) -> Vec<Diagnostic> {
	// OpenAPI carries JSON Schema verbatim, so it cannot lose a shape.
	if options.language == TargetLanguage::OpenApi {
		return Vec::new();
	}

	let published = spec.published(options.include_internal, &options.exclude);
	let mut diagnostics = Vec::new();

	for function in &published.functions {
		if let Some(reason) = skip_reason(function) {
			diagnostics.push(Diagnostic {
				target: options.language,
				path: format!("functions.{}", function.name),
				kind: DiagnosticKind::SkippedFunction,
				message: reason,
			});
		}
	}

	// Walk only what the generated client carries, so a loss inside a skipped
	// function's schemas is not reported.
	let usable = published.filtered(|function| skip_reason(function).is_none());
	let mut walker = Walker {
		target: options.language,
		spec: &usable,
		diagnostics,
	};

	for function in &usable.functions {
		walker.walk(
			&function.output_schema,
			&format!("functions.{}.output", function.name),
		);

		for param in &function.params {
			walker.walk(
				&param.schema,
				&format!("functions.{}.params.{}", function.name, param.name),
			);
		}
	}

	for channel in &usable.channels {
		walker.walk(
			&channel.args_schema,
			&format!("channels.{}.args", channel.name),
		);
		walker.walk(
			&channel.message_schema,
			&format!("channels.{}.message", channel.name),
		);
		walker.walk(
			&channel.presence_schema,
			&format!("channels.{}.presence", channel.name),
		);
	}

	for (name, schema) in &usable.schemas {
		walker.walk(schema, &format!("components.{name}"));
	}

	walker.diagnostics
}

struct Walker<'a> {
	target: TargetLanguage,
	spec: &'a ApiSpec,
	diagnostics: Vec<Diagnostic>,
}

impl Walker<'_> {
	fn loss(&mut self, path: &str, kind: DiagnosticKind, message: String) {
		self.diagnostics.push(Diagnostic {
			target: self.target,
			path: path.to_owned(),
			kind,
			message,
		});
	}

	fn walk(&mut self, schema: &Value, path: &str) {
		let analyzed = analyze(schema);

		if let Some(unsupported) = &analyzed.unsupported {
			let message = match unsupported {
				crate::schema::Unsupported::NonStringEnum => {
					format!(
						"non-string `enum` values are not modeled; {} only carries the base type",
						self.target
					)
				}
				crate::schema::Unsupported::NonStringConstant => {
					format!(
						"a non-string `const` is not modeled; {} only carries the base type",
						self.target
					)
				}
				other => {
					format!(
						"{}; {} renders it as `{}`",
						other.reason(),
						self.target,
						dynamic_type(self.target)
					)
				}
			};
			self.loss(path, DiagnosticKind::UnsupportedSchema, message);
		}

		match analyzed.shape {
			Shape::Ref(name) => {
				if self.spec.schema(name).is_none() {
					self.loss(
						path,
						DiagnosticKind::UnresolvedReference,
						format!(
							"`$ref` to `{name}` names no schema in the specification; {} renders \
							 it as `{}`",
							self.target,
							dynamic_type(self.target)
						),
					);
				}
			}
			Shape::Array { items } => {
				if let Some(items) = items {
					self.walk(items, &format!("{path}.items"));
				}
			}
			Shape::Tuple(items) => self.tuple(path, &items),
			Shape::Object(object) => self.object(path, &object),
			Shape::Map { values } => {
				if let Some(values) = values {
					self.walk(values, &format!("{path}.additionalProperties"));
				}
			}
			Shape::Union(variants) => self.union(schema, path, &variants),
			Shape::Integer {
				format: Some(format),
			} if matches!(format, "int64" | "uint64") => {
				// TypeScript always maps integers to `number`, which is an
				// IEEE-754 double; values beyond 2^53-1 round.
				if self.target == TargetLanguage::TypeScript {
					self.loss(
						path,
						DiagnosticKind::IntegerPrecision,
						format!(
							"`{format}` is rendered as `number`, which cannot represent integers \
							 beyond 2^53-1 exactly"
						),
					);
				}
			}
			Shape::Null
			| Shape::Boolean
			| Shape::Integer { .. }
			| Shape::Number { .. }
			| Shape::String { .. }
			| Shape::Any => {}
		}
	}

	fn tuple(&mut self, path: &str, items: &[&Value]) {
		match self.target {
			TargetLanguage::Go => {
				self.loss(
					path,
					DiagnosticKind::TupleFallback,
					"a fixed-length tuple becomes `[]any`; item types and length are lost"
						.to_owned(),
				);
				return;
			}
			TargetLanguage::Dart => {
				self.loss(
					path,
					DiagnosticKind::TupleFallback,
					"a fixed-length tuple becomes `List<dynamic>`; item types and length are lost"
						.to_owned(),
				);
				return;
			}
			_ => {}
		}

		for (index, item) in items.iter().enumerate() {
			self.walk(item, &format!("{path}.prefixItems[{index}]"));
		}
	}

	fn object(&mut self, path: &str, object: &crate::schema::ObjectShape<'_>) {
		for (property, property_schema) in &object.properties {
			let property_path = property_path(path, property);
			self.walk(property_schema, &property_path);

			// Dart has one nullable field for both absence and null, and
			// `toJson` omits it when null. A required nullable field therefore
			// sends absence where the schema requires an explicit null.
			if self.target == TargetLanguage::Dart
				&& object.is_required(property)
				&& !has_default(property_schema)
				&& analyze(property_schema).nullable
				&& !matches!(analyze(property_schema).shape, Shape::Null)
			{
				self.loss(
					&property_path,
					DiagnosticKind::NullableCollapsed,
					"a required nullable property becomes an optional Dart field whose `toJson` \
					 omits `null`; null and absence are indistinguishable"
						.to_owned(),
				);
			}
		}

		if let Some(additional) = object.additional {
			self.walk(additional, &format!("{path}.additionalProperties"));
		}
	}

	fn union(&mut self, schema: &Value, path: &str, variants: &[&Value]) {
		let fallback = match self.target {
			TargetLanguage::TypeScript | TargetLanguage::Python | TargetLanguage::OpenApi => false,
			TargetLanguage::Rust | TargetLanguage::Cli => recognize_enum(variants).is_none(),
			TargetLanguage::Go | TargetLanguage::Dart => true,
		};

		if fallback {
			self.loss(
				path,
				DiagnosticKind::UnionFallback,
				format!(
					"a union that is not a modeled serde enum becomes `{}`; its variants are no \
					 longer distinguishable",
					dynamic_type(self.target)
				),
			);
			return;
		}

		let keyword = if schema.get("anyOf").is_some() {
			"anyOf"
		} else {
			"oneOf"
		};

		for (index, variant) in variants.iter().enumerate() {
			self.walk(variant, &format!("{path}.{keyword}[{index}]"));
		}
	}
}

/// A property's path, bracket-quoting names that are not identifiers.
fn property_path(path: &str, property: &str) -> String {
	if !property.is_empty()
		&& property
			.chars()
			.all(|character| character.is_alphanumeric() || character == '_')
	{
		format!("{path}.properties.{property}")
	} else {
		format!("{path}.properties[{property:?}]")
	}
}

/// The type a target generates when it cannot model a shape.
const fn dynamic_type(target: TargetLanguage) -> &'static str {
	match target {
		TargetLanguage::TypeScript => "unknown",
		TargetLanguage::Python => "Any",
		TargetLanguage::Go => "any",
		TargetLanguage::Rust | TargetLanguage::Cli => "serde_json::Value",
		TargetLanguage::Dart => "dynamic",
		TargetLanguage::OpenApi => "the original schema",
	}
}
