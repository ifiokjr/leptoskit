//! Shared JSON Schema analysis for the generators.
//!
//! `schemars` emits JSON Schema 2020-12 shaped by `serde`'s conventions:
//! `type: [T, "null"]` and `anyOf: [T, {type: null}]` for `Option`, `const`
//! discriminators for internally tagged enums, single-key objects for
//! externally tagged ones. Every generator needs the same questions answered
//! about a schema node — what is it, is it nullable, which serde enum shape is
//! this — so they live here once, as a [`Shape`] view plus enum detectors.

use serde_json::Map;
use serde_json::Value;

use crate::spec::ref_name;

/// What a schema node describes, with nullability already separated out.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Shape<'a> {
	/// A reference to a named component schema.
	Ref(&'a str),
	/// Only `null`.
	Null,
	Boolean,
	Integer {
		format: Option<&'a str>,
	},
	Number {
		format: Option<&'a str>,
	},
	String {
		format: Option<&'a str>,
		/// `enum` values, when every one is a string.
		values: Option<Vec<&'a str>>,
		/// A `const` string.
		constant: Option<&'a str>,
	},
	Array {
		items: Option<&'a Value>,
	},
	/// `prefixItems`: a fixed-length heterogeneous list.
	Tuple(Vec<&'a Value>),
	Object(ObjectShape<'a>),
	/// Free-form keys: `additionalProperties` with no `properties`.
	Map {
		values: Option<&'a Value>,
	},
	/// `anyOf`/`oneOf`, null variants removed.
	Union(Vec<&'a Value>),
	/// `true`, `{}`, or a shape this crate does not model.
	Any,
}

/// An object schema's properties.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ObjectShape<'a> {
	/// Properties in schema order (alphabetical, as `serde_json` sorts them).
	pub properties: Vec<(&'a str, &'a Value)>,
	/// The `required` names.
	pub required: Vec<&'a str>,
	/// `additionalProperties`, when present and not `false`.
	pub additional: Option<&'a Value>,
}

impl ObjectShape<'_> {
	pub(crate) fn is_required(&self, name: &str) -> bool {
		self.required.contains(&name)
	}
}

/// Why a schema could not be turned into a shape the generators model.
///
/// [`Shape::Any`] is also the intentional unconstrained-JSON case (`true`,
/// `{}`, and annotation-only schemas). This reason is what separates the two:
/// it is `None` for intentional free-form JSON and `Some` when a construct
/// was dropped.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Unsupported<'a> {
	/// A `$ref` that does not name a component schema.
	ExternalReference(&'a str),
	/// `type` listed more than one non-null kind.
	MultipleTypes,
	/// `type` named a JSON type this crate does not model.
	UnknownType(&'a str),
	/// A composition such as `allOf` with more than one member.
	Composition { keyword: &'a str, parts: usize },
	/// A schema that mixes a non-string `enum`.
	NonStringEnum,
	/// A schema whose `const` is not a string.
	NonStringConstant,
	/// The schema is `false`, which admits no JSON value at all.
	FalseSchema,
	/// Keywords this crate does not understand leave the shape unknown.
	Unknown,
}

impl Unsupported<'_> {
	/// A sentence naming what was dropped, without a target's rendering.
	pub(crate) fn reason(&self) -> String {
		match self {
			Self::ExternalReference(reference) => {
				format!("`$ref` {reference:?} does not name a component schema")
			}
			Self::MultipleTypes => "`type` lists more than one non-null kind".to_owned(),
			Self::UnknownType(kind) => {
				format!("`type` names the unknown JSON type `{kind}`")
			}
			Self::Composition { keyword, parts } => {
				format!("`{keyword}` with {parts} members is not modeled")
			}
			Self::NonStringEnum => "non-string `enum` values are not modeled".to_owned(),
			Self::NonStringConstant => "a non-string `const` is not modeled".to_owned(),
			Self::FalseSchema => "the schema is `false`, which admits no value".to_owned(),
			Self::Unknown => "the schema uses keywords this generator does not model".to_owned(),
		}
	}
}

/// The analyzed node: its shape, whether `null` is allowed, and why the shape
/// is [`Shape::Any`] when that was not intentional.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Analyzed<'a> {
	pub shape: Shape<'a>,
	pub nullable: bool,
	pub unsupported: Option<Unsupported<'a>>,
}

/// Analyzes one schema node.
pub(crate) fn analyze(schema: &Value) -> Analyzed<'_> {
	let Some(object) = schema.as_object() else {
		return match schema {
			Value::Bool(true) => analyzed(Shape::Any, false, None),
			Value::Bool(false) => analyzed(Shape::Any, false, Some(Unsupported::FalseSchema)),
			_ => analyzed(Shape::Any, false, Some(Unsupported::Unknown)),
		};
	};

	if let Some(Value::String(reference)) = object.get("$ref")
		&& let Some(name) = ref_name(reference)
	{
		return analyzed(Shape::Ref(name), false, None);
	}

	let mut nullable = false;

	let kinds: Vec<&str> = match object.get("type") {
		Some(Value::String(kind)) => vec![kind.as_str()],
		Some(Value::Array(kinds)) => kinds.iter().filter_map(Value::as_str).collect(),
		_ => Vec::new(),
	};

	let kinds: Vec<&str> = kinds
		.into_iter()
		.filter(|kind| {
			if *kind == "null" {
				nullable = true;
				false
			} else {
				true
			}
		})
		.collect();

	for key in ["anyOf", "oneOf"] {
		if let Some(value) = object.get(key) {
			let Some(variants) = value.as_array() else {
				return analyzed(Shape::Any, nullable, Some(Unsupported::Unknown));
			};
			let rest: Vec<&Value> = variants
				.iter()
				.filter(|variant| {
					if is_null_schema(variant) {
						nullable = true;
						false
					} else {
						true
					}
				})
				.collect();

			return match rest.as_slice() {
				[] => analyzed(Shape::Null, true, None),
				[single] => {
					let inner = analyze(single);
					analyzed(inner.shape, nullable || inner.nullable, inner.unsupported)
				}
				_ => analyzed(Shape::Union(rest), nullable, None),
			};
		}
	}

	if let Some(Value::Array(parts)) = object.get("allOf")
		&& let [single] = parts.as_slice()
	{
		let inner = analyze(single);
		return analyzed(inner.shape, nullable || inner.nullable, inner.unsupported);
	}

	let format = object.get("format").and_then(Value::as_str);

	let shape = match kinds.as_slice() {
		[] if object.get("type").is_some() => Shape::Null,
		["boolean"] => Shape::Boolean,
		["integer"] => Shape::Integer { format },
		["number"] => Shape::Number { format },
		["string"] => {
			Shape::String {
				format,
				values: string_enum_values(object),
				constant: object.get("const").and_then(Value::as_str),
			}
		}
		["array"] => array_shape(object),
		["object"] => object_shape(object),
		[] => {
			// No `type`: infer from the other keywords.
			if object.contains_key("properties") {
				object_shape(object)
			} else if object.contains_key("items") || object.contains_key("prefixItems") {
				array_shape(object)
			} else if let Some(values) = string_enum_values(object) {
				Shape::String {
					format,
					values: Some(values),
					constant: None,
				}
			} else if let Some(constant) = object.get("const").and_then(Value::as_str) {
				Shape::String {
					format,
					values: None,
					constant: Some(constant),
				}
			} else if object.contains_key("additionalProperties") {
				object_shape(object)
			} else {
				Shape::Any
			}
		}
		_ => Shape::Any,
	};

	let unsupported = unsupported(object, &kinds);

	analyzed(shape, nullable, unsupported)
}

fn analyzed<'a>(
	shape: Shape<'a>,
	nullable: bool,
	unsupported: Option<Unsupported<'a>>,
) -> Analyzed<'a> {
	Analyzed {
		shape,
		nullable,
		unsupported,
	}
}

/// The construct a schema loses, when it has one, independent of its shape.
fn unsupported<'a>(object: &'a Map<String, Value>, kinds: &[&'a str]) -> Option<Unsupported<'a>> {
	// Diagnostics must not change the permissive generator's choice of type.
	// A foreign reference or unsupported composition can accompany a usable
	// base type; keep that type and report the part that was discarded.
	if let Some(Value::String(reference)) = object.get("$ref")
		&& ref_name(reference).is_none()
	{
		return Some(Unsupported::ExternalReference(reference));
	}
	if let Some(value) = object.get("allOf") {
		return Some(match value.as_array() {
			Some(parts) => {
				Unsupported::Composition {
					keyword: "allOf",
					parts: parts.len(),
				}
			}
			None => Unsupported::Unknown,
		});
	}

	if kinds.len() > 1 {
		return Some(Unsupported::MultipleTypes);
	}

	if let Some(kind) = kinds.first()
		&& !["boolean", "integer", "number", "string", "array", "object"].contains(kind)
	{
		return Some(Unsupported::UnknownType(kind));
	}

	if object.contains_key("enum") && string_enum_values(object).is_none() {
		return Some(Unsupported::NonStringEnum);
	}

	if object.contains_key("const") && object.get("const").and_then(Value::as_str).is_none() {
		return Some(Unsupported::NonStringConstant);
	}

	if has_unmodeled_keyword(object) {
		return Some(Unsupported::Unknown);
	}

	None
}

/// Keywords that constrain a value in ways this crate does not model.
fn has_unmodeled_keyword(object: &Map<String, Value>) -> bool {
	[
		"not",
		"if",
		"then",
		"else",
		"patternProperties",
		"dependentSchemas",
		"unevaluatedProperties",
		"unevaluatedItems",
	]
	.iter()
	.any(|keyword| object.contains_key(*keyword))
}

fn array_shape(object: &Map<String, Value>) -> Shape<'_> {
	if let Some(Value::Array(prefix)) = object.get("prefixItems")
		&& object
			.get("items")
			.is_none_or(|items| items == &Value::Bool(false))
	{
		return Shape::Tuple(prefix.iter().collect());
	}
	Shape::Array {
		items: object.get("items"),
	}
}

fn object_shape(object: &Map<String, Value>) -> Shape<'_> {
	let properties: Vec<(&str, &Value)> = object
		.get("properties")
		.and_then(Value::as_object)
		.map(|properties| {
			properties
				.iter()
				.map(|(name, schema)| (name.as_str(), schema))
				.collect()
		})
		.unwrap_or_default();

	let additional = match object.get("additionalProperties") {
		None | Some(Value::Bool(false)) => None,
		Some(schema) => Some(schema),
	};

	if properties.is_empty()
		&& !object.contains_key("properties")
		&& let Some(values) = additional
	{
		return Shape::Map {
			values: (values != &Value::Bool(true) && values != &Value::Object(Map::new()))
				.then_some(values),
		};
	}
	let required = object
		.get("required")
		.and_then(Value::as_array)
		.map(|names| names.iter().filter_map(Value::as_str).collect())
		.unwrap_or_default();
	Shape::Object(ObjectShape {
		properties,
		required,
		additional,
	})
}

fn string_enum_values(object: &Map<String, Value>) -> Option<Vec<&str>> {
	let values = object.get("enum")?.as_array()?;
	values.iter().map(Value::as_str).collect()
}

pub(crate) fn is_null_schema(schema: &Value) -> bool {
	schema
		.as_object()
		.is_some_and(|object| object.get("type") == Some(&Value::String("null".to_owned())))
}

/// The description a schema carries, if any.
pub(crate) fn description(schema: &Value) -> Option<&str> {
	schema.get("description").and_then(Value::as_str)
}

/// Whether a property schema carries a `default`, which makes it optional on
/// input.
pub(crate) fn has_default(schema: &Value) -> bool {
	schema.get("default").is_some()
}

/// One variant of a serde enum, however it is tagged.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum EnumVariant<'a> {
	/// No payload: a bare string externally, or only the tag internally.
	Unit { name: &'a str },
	/// A single unnamed payload.
	Newtype { name: &'a str, inner: &'a Value },
	/// Named fields.
	Struct {
		name: &'a str,
		fields: ObjectShape<'a>,
	},
}

impl EnumVariant<'_> {
	#[cfg_attr(not(test), allow(dead_code))]
	pub(crate) fn name(&self) -> &str {
		match self {
			Self::Unit { name } | Self::Newtype { name, .. } | Self::Struct { name, .. } => name,
		}
	}
}

/// A serde enum recognized from a union of schemas.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SerdeEnum<'a> {
	/// `#[serde(tag = "...")]`.
	Internal {
		tag: &'a str,
		variants: Vec<EnumVariant<'a>>,
	},
	/// `#[serde(tag = "...", content = "...")]`.
	Adjacent {
		tag: &'a str,
		content: &'a str,
		variants: Vec<EnumVariant<'a>>,
	},
	/// serde's default representation.
	External { variants: Vec<EnumVariant<'a>> },
}

/// Recognizes the serde enum behind a union of variant schemas, if it is one.
pub(crate) fn recognize_enum<'a>(variants: &[&'a Value]) -> Option<SerdeEnum<'a>> {
	if variants.is_empty() {
		return None;
	}

	if let Some(tagged) = recognize_tagged(variants) {
		return Some(tagged);
	}

	recognize_external(variants)
}

/// Internally or adjacently tagged: every variant is an object with one
/// shared `const` string property.
fn recognize_tagged<'a>(variants: &[&'a Value]) -> Option<SerdeEnum<'a>> {
	let objects: Vec<ObjectShape<'_>> = variants
		.iter()
		.map(|variant| {
			match analyze(variant).shape {
				Shape::Object(object) => Some(object),
				_ => None,
			}
		})
		.collect::<Option<_>>()?;
	let first = objects.first()?;
	let tag = first
		.properties
		.iter()
		.find(|(name, schema)| {
			first.is_required(name)
				&& const_string(schema).is_some()
				&& objects.iter().all(|object| {
					object.is_required(name)
						&& object
							.properties
							.iter()
							.any(|(other, schema)| other == name && const_string(schema).is_some())
				})
		})
		.map(|(name, _)| *name)?;
	// Distinct tag values, or the discriminator is not one.
	let mut seen = Vec::new();

	for object in &objects {
		let value = object
			.properties
			.iter()
			.find(|(name, _)| *name == tag)
			.and_then(|(_, schema)| const_string(schema))?;

		if seen.contains(&value) {
			return None;
		}

		seen.push(value);
	}

	// Adjacent: every other property shares one name and is the payload.
	let other_names: Vec<&str> = objects
		.iter()
		.flat_map(|object| {
			object
				.properties
				.iter()
				.map(|(name, _)| *name)
				.filter(|name| *name != tag)
		})
		.collect();
	let mut distinct = other_names.clone();
	distinct.sort_unstable();
	distinct.dedup();
	let payload_is_uniform =
		distinct.len() == 1 && objects.iter().all(|object| object.properties.len() <= 2);

	if payload_is_uniform && objects.iter().any(|object| object.properties.len() == 2) {
		let content = distinct[0];
		let variants = objects
			.iter()
			.zip(&seen)
			.map(|(object, name)| {
				match object.properties.iter().find(|(prop, _)| *prop == content) {
					Some((_, inner)) => EnumVariant::Newtype { name, inner },
					None => EnumVariant::Unit { name },
				}
			})
			.collect();
		return Some(SerdeEnum::Adjacent {
			tag,
			content,
			variants,
		});
	}

	let variants = objects
		.into_iter()
		.zip(seen)
		.map(|(object, name)| {
			let fields = ObjectShape {
				properties: object
					.properties
					.into_iter()
					.filter(|(prop, _)| *prop != tag)
					.collect(),
				required: object
					.required
					.into_iter()
					.filter(|prop| *prop != tag)
					.collect(),
				additional: object.additional,
			};

			if fields.properties.is_empty() {
				EnumVariant::Unit { name }
			} else {
				EnumVariant::Struct { name, fields }
			}
		})
		.collect();
	Some(SerdeEnum::Internal { tag, variants })
}

/// Externally tagged: string-enum members are unit variants and single-key
/// closed objects carry a payload under the variant name.
fn recognize_external<'a>(variants: &[&'a Value]) -> Option<SerdeEnum<'a>> {
	let mut out = Vec::new();

	for variant in variants {
		match analyze(variant).shape {
			Shape::String {
				values: Some(names),
				..
			} => {
				out.extend(names.into_iter().map(|name| EnumVariant::Unit { name }));
			}
			Shape::String {
				constant: Some(name),
				..
			} => out.push(EnumVariant::Unit { name }),
			Shape::Object(object)
				if object.properties.len() == 1
					&& object.additional.is_none()
					&& variant
						.get("additionalProperties")
						.is_some_and(|extra| extra == &Value::Bool(false)) =>
			{
				let (name, inner) = object.properties[0];

				match analyze(inner).shape {
					Shape::Object(fields) if !fields.properties.is_empty() => {
						out.push(EnumVariant::Struct { name, fields });
					}

					_ => out.push(EnumVariant::Newtype { name, inner }),
				}
			}
			_ => return None,
		}
	}

	Some(SerdeEnum::External { variants: out })
}

/// The `const` string of a schema, or the single value of a one-string enum.
fn const_string(schema: &Value) -> Option<&str> {
	let object = schema.as_object()?;

	if let Some(constant) = object.get("const").and_then(Value::as_str) {
		return Some(constant);
	}

	match object.get("enum")?.as_array()?.as_slice() {
		[Value::String(single)] => Some(single),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	#[test]
	fn separates_nullability_from_shape() {
		let typed = json!({ "type": ["integer", "null"], "format": "uint32" });
		assert_eq!(
			analyze(&typed),
			Analyzed {
				shape: Shape::Integer {
					format: Some("uint32")
				},
				nullable: true,
				unsupported: None,
			}
		);
		let any_of =
			json!({ "anyOf": [{ "$ref": "#/components/schemas/Todo" }, { "type": "null" }] });
		assert_eq!(
			analyze(&any_of),
			Analyzed {
				shape: Shape::Ref("Todo"),
				nullable: true,
				unsupported: None,
			}
		);
		assert_eq!(analyze(&json!({ "type": "null" })).shape, Shape::Null);
		assert_eq!(analyze(&json!(true)).shape, Shape::Any);
		assert_eq!(analyze(&json!({})).shape, Shape::Any);
		assert_eq!(
			analyze(&json!(true)).unsupported,
			None,
			"an unconstrained schema is intentional"
		);
		assert_eq!(analyze(&json!({})).unsupported, None);
	}

	#[test]
	fn infers_shapes_without_a_type_keyword() {
		assert!(matches!(
			analyze(&json!({ "properties": { "a": { "type": "string" } } })).shape,
			Shape::Object(_)
		));
		assert!(matches!(
			analyze(&json!({ "items": { "type": "string" } })).shape,
			Shape::Array { items: Some(_) }
		));
		assert!(matches!(
			analyze(&json!({ "prefixItems": [{ "type": "string" }] })).shape,
			Shape::Tuple(_)
		));
		assert!(matches!(
			analyze(&json!({ "enum": ["a", "b"] })).shape,
			Shape::String {
				values: Some(_),
				..
			}
		));
		assert!(matches!(
			analyze(&json!({ "const": "only" })).shape,
			Shape::String {
				constant: Some("only"),
				..
			}
		));
		assert!(matches!(
			analyze(&json!({ "additionalProperties": { "type": "integer" } })).shape,
			Shape::Map { values: Some(_) }
		));
		assert!(matches!(
			analyze(&json!({ "additionalProperties": true })).shape,
			Shape::Map { values: None }
		));
		assert!(matches!(
			analyze(&json!({ "additionalProperties": {} })).shape,
			Shape::Map { values: None }
		));

		assert_eq!(
			analyze(&json!({ "minimum": 1 })).shape,
			Shape::Any,
			"no shape keywords at all"
		);
		assert_eq!(
			analyze(&json!({ "minimum": 1 })).unsupported,
			None,
			"constraints alone still describe any JSON value"
		);
	}

	#[test]
	fn diagnostics_preserve_usable_types_beside_unsupported_constructs() {
		for schema in [
			json!({ "type": "string", "$ref": "https://example.invalid/type.json" }),
			json!({ "type": "string", "allOf": [{ "minLength": 1 }, { "maxLength": 10 }] }),
		] {
			let result = analyze(&schema);
			assert!(matches!(result.shape, Shape::String { .. }));
			assert!(result.unsupported.is_some());
		}
	}

	#[test]
	fn separates_unconstrained_json_from_unmodeled_constructs() {
		for unconstrained in [
			json!(true),
			json!({}),
			json!({ "description": "anything" }),
			json!({ "type": "object", "additionalProperties": true }),
			json!({ "type": "array" }),
		] {
			assert_eq!(
				analyze(&unconstrained).unsupported,
				None,
				"{unconstrained} is intentionally free-form"
			);
		}

		for (schema, expected) in [
			(json!(false), Unsupported::FalseSchema),
			(
				json!({ "type": ["string", "integer"] }),
				Unsupported::MultipleTypes,
			),
			(
				json!({ "type": "string", "enum": ["a", 1] }),
				Unsupported::NonStringEnum,
			),
			(json!({ "const": 42 }), Unsupported::NonStringConstant),
			(json!({ "type": "date" }), Unsupported::UnknownType("date")),
			(json!({ "not": { "type": "string" } }), Unsupported::Unknown),
			(
				json!({
					"allOf": [{ "type": "string" }, { "type": "string" }],
				}),
				Unsupported::Composition {
					keyword: "allOf",
					parts: 2,
				},
			),
		] {
			assert_eq!(analyze(&schema).unsupported, Some(expected), "{schema}");
		}
	}

	#[test]
	fn handles_unions_of_nulls_and_combinators() {
		let only_null = json!({ "anyOf": [{ "type": "null" }] });
		assert_eq!(
			analyze(&only_null),
			Analyzed {
				shape: Shape::Null,
				nullable: true,
				unsupported: None,
			}
		);
		let one_of = json!({ "oneOf": [{ "type": "string" }, { "type": "integer" }] });

		assert!(matches!(analyze(&one_of).shape, Shape::Union(variants) if variants.len() == 2));
		let all_of = json!({ "allOf": [{ "type": "boolean" }] });
		assert_eq!(analyze(&all_of).shape, Shape::Boolean);
		let many = json!({ "type": ["string", "integer"] });

		assert_eq!(analyze(&many).shape, Shape::Any);
		assert_eq!(analyze(&many).unsupported, Some(Unsupported::MultipleTypes));
		let composed = json!({ "allOf": [{ "type": "string" }, { "type": "string" }] });

		assert_eq!(analyze(&composed).shape, Shape::Any);
		assert_eq!(
			analyze(&composed).unsupported,
			Some(Unsupported::Composition {
				keyword: "allOf",
				parts: 2
			})
		);

		let foreign_ref = json!({ "$ref": "https://elsewhere/schema.json" });
		assert_eq!(analyze(&foreign_ref).shape, Shape::Any);
		assert!(matches!(
			analyze(&foreign_ref).unsupported,
			Some(Unsupported::ExternalReference(_))
		));
		let defs_ref = json!({ "$ref": "#/$defs/Thing" });
		assert_eq!(analyze(&defs_ref).shape, Shape::Ref("Thing"));
		let closed = json!({ "type": "object", "additionalProperties": false });
		assert!(matches!(
			analyze(&closed).shape,

			Shape::Object(ObjectShape { properties, additional: None, .. }) if properties.is_empty()
		));
		let array_with_items = json!({ "type": "array", "prefixItems": [{ "type": "string" }], "items": { "type": "string" } });
		assert!(matches!(
			analyze(&array_with_items).shape,
			Shape::Array { .. }
		));
	}

	#[test]
	fn recognizes_maps_tuples_and_string_enums() {
		let map = json!({ "type": "object", "additionalProperties": { "type": "integer" } });
		assert!(matches!(
			analyze(&map).shape,
			Shape::Map { values: Some(_) }
		));
		let tuple = json!({ "type": "array", "prefixItems": [{ "type": "string" }, { "type": "integer" }], "items": false });

		assert!(matches!(analyze(&tuple).shape, Shape::Tuple(items) if items.len() == 2));
		let values = json!({ "type": "string", "enum": ["all", "done"] });
		assert!(matches!(
			analyze(&values).shape,

			Shape::String { values: Some(values), .. } if values == ["all", "done"]
		));
	}

	#[test]
	fn recognizes_internally_tagged_enums() {
		let created = json!({ "type": "object", "properties": { "kind": { "type": "string", "const": "created" }, "id": { "type": "integer" } }, "required": ["kind", "id"] });
		let deleted = json!({ "type": "object", "properties": { "kind": { "type": "string", "const": "deleted" }, "id": { "type": "integer" }, "reason": { "type": "string" } }, "required": ["kind", "id"] });
		let Some(SerdeEnum::Internal { tag, variants }) = recognize_enum(&[&created, &deleted])
		else {
			panic!("internal enum");
		};
		assert_eq!(tag, "kind");
		assert!(
			matches!(&variants[0], EnumVariant::Struct { name: "created", fields } if fields.properties.len() == 1)
		);
		assert!(matches!(
			&variants[1],
			EnumVariant::Struct {
				name: "deleted",
				..
			}
		));
	}

	#[test]
	fn single_field_variants_are_adjacent_on_the_wire() {
		// `Internal { id }` and `Adjacent(id)` serialize identically, so a
		// shared content name classifies as adjacently tagged; both generate
		// the same JSON either way.
		let created = json!({ "type": "object", "properties": { "kind": { "type": "string", "const": "created" }, "id": { "type": "integer" } }, "required": ["kind", "id"] });
		let deleted = json!({ "type": "object", "properties": { "kind": { "type": "string", "const": "deleted" } }, "required": ["kind"] });
		let Some(SerdeEnum::Adjacent { tag, content, .. }) = recognize_enum(&[&created, &deleted])
		else {
			panic!("adjacent enum");
		};
		assert_eq!((tag, content), ("kind", "id"));
	}

	#[test]
	fn recognizes_adjacently_tagged_enums() {
		let text = json!({ "type": "object", "properties": { "t": { "type": "string", "const": "Text" }, "c": { "type": "string" } }, "required": ["t", "c"] });
		let quit = json!({ "type": "object", "properties": { "t": { "type": "string", "const": "Quit" } }, "required": ["t"] });
		let Some(SerdeEnum::Adjacent {
			tag,
			content,
			variants,
		}) = recognize_enum(&[&text, &quit])
		else {
			panic!("adjacent enum");
		};
		assert_eq!((tag, content), ("t", "c"));
		assert!(matches!(
			&variants[0],
			EnumVariant::Newtype { name: "Text", .. }
		));
		assert!(matches!(&variants[1], EnumVariant::Unit { name: "Quit" }));
	}

	#[test]
	fn recognizes_externally_tagged_enums() {
		let units = json!({ "type": "string", "enum": ["All", "None"] });
		let newtype = json!({ "type": "object", "properties": { "Limit": { "type": "integer" } }, "required": ["Limit"], "additionalProperties": false });
		let structy = json!({ "type": "object", "properties": { "Range": { "type": "object", "properties": { "from": { "type": "integer" } }, "required": ["from"] } }, "required": ["Range"], "additionalProperties": false });
		let Some(SerdeEnum::External { variants }) = recognize_enum(&[&units, &newtype, &structy])
		else {
			panic!("external enum");
		};
		let names: Vec<&str> = variants.iter().map(EnumVariant::name).collect();
		assert_eq!(names, ["All", "None", "Limit", "Range"]);

		assert!(matches!(variants[2], EnumVariant::Newtype { .. }));
		assert!(matches!(variants[3], EnumVariant::Struct { .. }));
	}

	#[test]
	fn arbitrary_unions_are_not_enums() {
		let a = json!({ "type": "string" });
		let b = json!({ "type": "integer" });
		assert_eq!(recognize_enum(&[&a, &b]), None);
	}
}
