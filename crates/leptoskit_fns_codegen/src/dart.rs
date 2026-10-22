//! The Dart client: a package with typed models over `package:http`.
//!
//! Models are ordinary classes with `fromJson`/`toJson` generated from the
//! schemas, string enums become Dart enums, and shapes Dart cannot model yet
//! (tagged unions) arrive as `Object?` holding the decoded JSON. Calls go to
//! the same routes the app itself uses.

// Generators build source text; the allocation `write!` would save is noise here.
#![allow(clippy::format_push_string)]

use std::collections::HashMap;

use serde_json::Value;

use crate::generate::Context;
use crate::generate::Generated;
use crate::generate::GeneratedFile;
use crate::naming::dart_identifier;
use crate::naming::dart_package_name;
use crate::naming::dart_string;
use crate::naming::type_name;
use crate::schema::Shape;
use crate::schema::analyze;
use crate::schema::description;
use crate::schema::has_default;
use crate::spec::ApiSpec;

/// Names the generated runtime owns; components colliding with them get a
/// suffix.
const RESERVED: &[&str] = &["Client", "FnError", "defaultServerUrl", "toQueryString"];

/// Whether a named type is a class or an enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NamedKind {
	Class,
	Enum,
}

/// A Dart type, carrying just enough to encode and decode it.
#[derive(Debug, Clone, PartialEq)]
enum DartType {
	/// The `()` of a function returning nothing.
	Void,
	Bool,
	Int,
	Double,
	String,
	Named(String, NamedKind),
	List(Box<DartType>),
	Map(Box<DartType>),
	Dynamic,
	Nullable(Box<DartType>),
}

impl DartType {
	fn render(&self) -> String {
		match self {
			Self::Void => "void".to_owned(),
			Self::Bool => "bool".to_owned(),
			Self::Int => "int".to_owned(),
			Self::Double => "double".to_owned(),
			Self::String => "String".to_owned(),
			Self::Named(name, _) => name.clone(),
			Self::List(inner) => format!("List<{}>", inner.render()),
			Self::Map(inner) => format!("Map<String, {}>", inner.render()),
			Self::Dynamic => "Object?".to_owned(),
			Self::Nullable(inner) => format!("{}?", inner.render()),
		}
	}

	/// An expression converting the JSON value `expr` into this type.
	fn decode(&self, expr: &str) -> String {
		match self {
			Self::Void | Self::Dynamic => expr.to_owned(),
			Self::Bool => format!("{expr} as bool"),
			Self::Int => format!("({expr} as num).toInt()"),
			Self::Double => format!("({expr} as num).toDouble()"),
			Self::String => format!("{expr} as String"),
			Self::Named(name, NamedKind::Class) => {
				format!("{name}.fromJson({expr} as Map<String, dynamic>)")
			}
			Self::Named(name, NamedKind::Enum) => format!("{name}.fromJson({expr})"),
			Self::List(inner) => {
				format!(
					"({expr} as List<dynamic>).map((item) => {}).toList()",
					inner.decode("item")
				)
			}
			Self::Map(inner) => {
				format!(
					"({expr} as Map<String, dynamic>).map((key, value) => MapEntry(key, {}))",
					inner.decode("value")
				)
			}
			Self::Nullable(inner) => {
				format!("{expr} == null ? null : {}", inner.decode(expr))
			}
		}
	}

	/// An expression converting the Dart value `expr` into JSON.
	fn encode(&self, expr: &str) -> String {
		match self {
			Self::Void | Self::Dynamic | Self::Bool | Self::Int | Self::Double | Self::String => {
				expr.to_owned()
			}
			Self::Named(..) => format!("{expr}.toJson()"),
			Self::List(inner) => {
				let item = inner.encode("item");
				if item == "item" {
					expr.to_owned()
				} else {
					format!("{expr}.map((item) => {item}).toList()")
				}
			}
			Self::Map(inner) => {
				let value = inner.encode("value");
				if value == "value" {
					expr.to_owned()
				} else {
					format!("{expr}.map((key, value) => MapEntry(key, {value}))")
				}
			}
			Self::Nullable(inner) => inner.encode(expr),
		}
	}
}

/// Accumulates the generated classes and enums, and the names already taken.
#[derive(Default)]
struct DartTypes {
	taken: Vec<String>,
	kinds: HashMap<String, NamedKind>,
	/// Emitted model declarations, in creation order.
	models: Vec<String>,
}

impl DartTypes {
	fn reserve(&mut self, wanted: &str) -> String {
		let mut name = wanted.to_owned();
		let mut counter = 2;
		while self.taken.contains(&name) || RESERVED.contains(&name.as_str()) {
			name = format!("{wanted}{counter}");
			counter += 1;
		}
		self.taken.push(name.clone());
		name
	}

	/// The Dart type for a schema, synthesizing classes and enums for
	/// anonymous shapes, named after `hint`.
	fn convert(&mut self, schema: &Value, hint: &str, spec: &ApiSpec) -> DartType {
		let analyzed = analyze(schema);
		let nullable = analyzed.nullable;
		let base = match analyzed.shape {
			Shape::Ref(name) => {
				if let Some(kind) = self.kinds.get(name) {
					DartType::Named(type_name(name), *kind)
				} else {
					match spec.schema(name) {
						Some(component) => self.convert(component, &type_name(name), spec),
						None => DartType::Dynamic,
					}
				}
			}
			Shape::Null => DartType::Void,
			Shape::Boolean => DartType::Bool,
			Shape::Integer { .. } => DartType::Int,
			Shape::Number { .. } => DartType::Double,
			Shape::String {
				values: Some(_), ..
			} => {
				let name = self.reserve(&type_name(hint));
				let declaration = enum_declaration(schema, &name);
				self.kinds.insert(name.clone(), NamedKind::Enum);
				self.models.push(declaration);
				DartType::Named(name, NamedKind::Enum)
			}
			Shape::String { .. } => DartType::String,
			Shape::Array { items } => {
				DartType::List(Box::new(items.map_or(DartType::Dynamic, |items| {
					self.convert(items, &format!("{hint}Item"), spec)
				})))
			}
			Shape::Tuple(_) => DartType::List(Box::new(DartType::Dynamic)),
			Shape::Object(object) => {
				if object.properties.is_empty() {
					DartType::Map(Box::new(
						object.additional.map_or(DartType::Dynamic, |values| {
							self.convert(values, &format!("{hint}Value"), spec)
						}),
					))
				} else {
					let name = self.reserve(&type_name(hint));
					let declaration = class_declaration(schema, &name, &object, self, spec);
					self.kinds.insert(name.clone(), NamedKind::Class);
					self.models.push(declaration);
					DartType::Named(name, NamedKind::Class)
				}
			}
			Shape::Map { values } => {
				DartType::Map(Box::new(values.map_or(DartType::Dynamic, |values| {
					self.convert(values, &format!("{hint}Value"), spec)
				})))
			}
			Shape::Union(_) | Shape::Any => DartType::Dynamic,
		};
		// `null` alone is `void`, not an optional nothing.
		if nullable && base != DartType::Void {
			DartType::Nullable(Box::new(base))
		} else {
			base
		}
	}
}

/// One field of a generated class.
struct Field {
	dart: String,
	wire: String,
	ty: DartType,
	nullable: bool,
	docs: Vec<String>,
}

/// The `class` declaration for an object schema.
fn class_declaration(
	schema: &Value,
	name: &str,
	object: &crate::schema::ObjectShape<'_>,
	types: &mut DartTypes,
	spec: &ApiSpec,
) -> String {
	let mut fields = Vec::new();
	for (wire, property) in &object.properties {
		let ty = types.convert(property, &format!("{name}{}", type_name(wire)), spec);
		let nullable = !object.is_required(wire)
			|| has_default(property)
			|| matches!(ty, DartType::Nullable(_));
		fields.push(Field {
			dart: dart_identifier(wire),
			wire: (*wire).to_owned(),
			ty,
			nullable,
			docs: description(property)
				.map(|docs| docs.lines().map(str::trim).map(str::to_owned).collect())
				.unwrap_or_default(),
		});
	}

	let mut out = String::new();
	push_docs(&mut out, description(schema), "");
	out.push_str(&format!("class {name} {{\n"));
	out.push_str(&format!(
		"\t{name}({{\n{}\n\t}});\n\n",
		fields
			.iter()
			.map(|field| {
				if field.nullable {
					format!("\t\tthis.{},", field.dart)
				} else {
					format!("\t\trequired this.{},", field.dart)
				}
			})
			.collect::<Vec<_>>()
			.join("\n")
	));
	for field in &fields {
		let ty = if field.nullable {
			DartType::Nullable(Box::new(field.ty.clone()))
		} else {
			field.ty.clone()
		};
		for line in &field.docs {
			out.push_str(&format!("\t/// {line}\n"));
		}
		out.push_str(&format!("\tfinal {} {};\n", ty.render(), field.dart));
	}
	out.push_str(&format!(
		"\n\tfactory {name}.fromJson(Map<String, dynamic> json) => {name}(\n{}\n\t);\n",
		fields
			.iter()
			.map(|field| {
				let ty = if field.nullable {
					DartType::Nullable(Box::new(field.ty.clone()))
				} else {
					field.ty.clone()
				};
				format!(
					"\t\t{}: {},",
					field.dart,
					ty.decode(&format!("json[{}]", dart_string(&field.wire)))
				)
			})
			.collect::<Vec<_>>()
			.join("\n")
	));
	out.push_str(&format!(
		"\n\tMap<String, Object?> toJson() => <String, Object?>{{\n{}\n\t}};\n",
		fields
			.iter()
			.map(|field| {
				let entry = if field.nullable {
					DartType::Nullable(Box::new(field.ty.clone())).encode(&field.dart)
				} else {
					field.ty.encode(&field.dart)
				};
				if field.nullable {
					format!(
						"\t\tif ({0} != null) {1}: {2},",
						field.dart,
						dart_string(&field.wire),
						entry
					)
				} else {
					format!("\t\t{}: {},", dart_string(&field.wire), entry)
				}
			})
			.collect::<Vec<_>>()
			.join("\n")
	));
	out.push_str("}\n\n");
	out
}

/// The `enum` declaration for a string-enum schema.
fn enum_declaration(schema: &Value, name: &str) -> String {
	let values: Vec<&str> = match analyze(schema).shape {
		Shape::String {
			values: Some(values),
			..
		} => values,
		_ => Vec::new(),
	};
	let mut out = String::new();
	push_docs(&mut out, description(schema), "");
	out.push_str(&format!("enum {name} {{\n"));
	let mut variants = Vec::new();
	for (index, value) in values.iter().enumerate() {
		let variant = dart_identifier(value);
		// Enum members Dart defines, which a value may not shadow.
		let variant = if ["values", "wire", "index", "name", "toJson", "fromJson"]
			.contains(&variant.as_str())
		{
			format!("{variant}_")
		} else {
			variant
		};
		// Values differing only in case still need distinct variant names.
		let variant = if values
			.iter()
			.enumerate()
			.any(|(other, candidate)| other != index && dart_identifier(candidate) == variant)
		{
			format!("{variant}{index}")
		} else {
			variant
		};
		variants.push(format!("\t{}({})", variant, dart_string(value)));
	}
	out.push_str(&variants.join(",\n"));
	out.push_str(";\n");
	out.push_str(&format!(
		"\tconst {name}(this.wire);\n\n\tfinal String wire;\n\n\tfactory {name}.fromJson(Object? \
		 json) => values.firstWhere((value) => value.wire == json);\n\n\tString toJson() => \
		 wire;\n}}\n\n",
	));
	out
}

/// Pushes a `///` doc comment at `indent`.
fn push_docs(out: &mut String, docs: Option<&str>, indent: &str) {
	let Some(docs) = docs else {
		return;
	};
	let docs = docs.trim();
	if docs.is_empty() {
		return;
	}
	let safe = docs.replace("*/", "*\\/");
	for line in safe.lines() {
		if line.is_empty() {
			out.push_str(&format!("{indent}///\n"));
		} else {
			out.push_str(&format!("{indent}/// {line}\n"));
		}
	}
}

pub(crate) fn generate(context: &Context<'_>) -> Generated {
	let package = dart_package_name(&context.options.package_name);
	let mut types = DartTypes::default();
	// Reserve the component names first, so references and synthesized types
	// cannot collide with them.
	let components: Vec<(String, &Value)> = context
		.spec
		.schemas
		.iter()
		.map(|(name, schema)| {
			let dart_name = types.reserve(&type_name(name));
			let kind = match analyze(schema).shape {
				Shape::Object(ref object) if !object.properties.is_empty() => {
					Some(NamedKind::Class)
				}
				Shape::String {
					values: Some(_), ..
				} => Some(NamedKind::Enum),
				_ => None,
			};
			if let Some(kind) = kind {
				types.kinds.insert(dart_name.clone(), kind);
			}
			(dart_name, schema)
		})
		.collect();

	let mut models = String::new();
	for (dart_name, schema) in &components {
		let name = type_name(dart_name);
		match analyze(schema).shape {
			Shape::Object(ref object) if !object.properties.is_empty() => {
				models.push_str(&class_declaration(
					schema,
					&name,
					object,
					&mut types,
					context.spec,
				));
			}
			Shape::String {
				values: Some(_), ..
			} => {
				models.push_str(&enum_declaration(schema, &name));
			}
			_ => {}
		}
	}

	// The client's signatures may synthesize more models (inline enums,
	// anonymous argument objects), so it is rendered before the models are
	// written out.
	let client = client_class(context, &mut types);
	let mut library = context.banner("//");
	library.push_str("library;\n\nimport 'dart:convert';\nimport 'dart:typed_data';\n\n");
	library.push_str("import 'package:http/http.dart' as http;\n\n");
	library.push_str(&models);
	for synthesized in &types.models {
		library.push_str(synthesized);
	}
	library.push_str(RUNTIME);
	library.push_str(&client);

	Generated {
		files: vec![
			GeneratedFile::scaffold("pubspec.yaml", pubspec(context, &package)),
			GeneratedFile::code(format!("lib/{package}.dart"), library),
			GeneratedFile::scaffold("README.md", readme(context)),
		],
	}
}

/// The generated `Client`, with one method per published function.
fn client_class(context: &Context<'_>, types: &mut DartTypes) -> String {
	let mut out = String::new();
	out.push_str(&format!(
		"/// The server origin baked in at generation time; `null` means the client's own \
		 page.\nconst String? defaultServerUrl = {};\n\n",
		context
			.server_url
			.as_deref()
			.map_or_else(|| "null".to_owned(), dart_string)
	));
	out.push_str("/// A typed client over `package:http`.\n");
	out.push_str("class Client {\n");
	out.push_str(
		"\t/// Creates a client for [serverUrl], an origin such as\n\t/// \
		 `https://api.example.com`; defaults to [defaultServerUrl]. Pass\n\t/// [httpClient] to \
		 share connections.\n\tClient({String? serverUrl, http.Client? httpClient})\n\t\t : \
		 serverUrl = serverUrl ?? defaultServerUrl,\n\t\t   _http = httpClient ?? \
		 http.Client();\n\n\t/// The origin the client sends requests to, without a trailing \
		 slash.\n\tfinal String? serverUrl;\n\n\tfinal http.Client _http;\n",
	);
	for function in context.functions() {
		out.push('\n');
		let mut docs = function.docs.clone();
		let documented: Vec<&crate::spec::ApiParam> = function
			.params
			.iter()
			.filter(|param| !param.docs.is_empty())
			.collect();
		if !documented.is_empty() {
			if !docs.is_empty() {
				docs.push_str("\n\n");
			}
			docs.push_str("Arguments:\n");
			for param in documented {
				docs.push_str(&format!(
					"- `{}`: {}\n",
					dart_identifier(&param.name),
					param.docs.trim()
				));
			}
			docs.pop();
		}
		if function.deprecated {
			if docs.is_empty() {
				docs.push_str("Deprecated.");
			} else {
				docs.push_str("\n\nDeprecated.");
			}
		}
		push_docs(&mut out, (!docs.is_empty()).then_some(docs.as_str()), "\t");

		let name = dart_identifier(&function.name);
		let output = types.convert(
			&function.output_schema,
			&format!("{}Result", type_name(&function.name)),
			context.spec,
		);

		let mut parameters = Vec::new();
		let mut payload = Vec::new();
		for param in &function.params {
			let ty = types.convert(
				&param.schema,
				&format!("{}{}", type_name(&function.name), type_name(&param.name)),
				context.spec,
			);
			let identifier = dart_identifier(&param.name);
			// The parameter type is what the wire accepts; an optional
			// non-nullable argument becomes nullable once, not twice.
			let optional = !param.is_required() || matches!(ty, DartType::Nullable(_));
			let parameter_type = match (&ty, optional) {
				(DartType::Nullable(_), _) | (_, false) => ty.clone(),
				(_, true) => DartType::Nullable(Box::new(ty.clone())),
			};
			parameters.push(if optional {
				format!("{} {}", parameter_type.render(), identifier)
			} else {
				format!("required {} {}", parameter_type.render(), identifier)
			});
			if optional {
				payload.push(format!(
					"\t\t\t\tif ({identifier} != null) {}: {},",
					dart_string(&param.name),
					ty.encode(&identifier)
				));
			} else {
				payload.push(format!(
					"\t\t\t\t{}: {},",
					dart_string(&param.name),
					ty.encode(&identifier)
				));
			}
		}

		let returns = matches!(output, DartType::Void);
		let return_type = if returns {
			"Future<void>".to_owned()
		} else {
			format!("Future<{}>", output.render())
		};
		let parameter_list = if parameters.is_empty() {
			String::new()
		} else {
			format!("{{\n\t\t{}\n\t}}", parameters.join(",\n\t\t"))
		};
		let binding = if returns { "" } else { "final result = " };
		let decode = if returns {
			String::new()
		} else {
			format!("\n\t\treturn {};", output.decode("result"))
		};
		let payload_map = if payload.is_empty() {
			"const <String, Object?>{}".to_owned()
		} else {
			format!(
				"\n\t\t\t\t<String, Object?>{{\n{}\n\t\t\t\t}}",
				payload.join("\n")
			)
		};
		out.push_str(&format!(
			"\t{return_type} {name}({parameter_list}) async {{\n\t\t{binding}await _invoke({}, \
			 {}, {}, {payload_map});{decode}\n\t}}\n",
			dart_string(&function.method),
			dart_string(&function.path),
			dart_string(match function.input {
				crate::spec::HttpInput::Query => "query",
				crate::spec::HttpInput::Form => "form",
				crate::spec::HttpInput::Json | crate::spec::HttpInput::Other { .. } => "json",
			}),
		));
	}
	out.push_str(
		"\n\tFuture<Object?> _invoke(\n\t\tString method,\n\t\tString path,\n\t\tString \
		 encoding,\n\t\tMap<String, Object?> payload,\n\t) async {\n\t\tvar url = '${serverUrl ?? \
		 ''}$path';\n\t\tfinal headers = <String, String>{'Accept': \
		 'application/json'};\n\t\tUint8List? body;\n\t\tif (encoding == 'query') {\n\t\t\tfinal \
		 query = toQueryString(payload);\n\t\t\tif (query.isNotEmpty) {\n\t\t\t\turl += \
		 (url.contains('?') ? '&' : '?') + query;\n\t\t\t}\n\t\t} else if (encoding == 'form') \
		 {\n\t\t\theaders['Content-Type'] = 'application/x-www-form-urlencoded';\n\t\t\tbody = \
		 Uint8List.fromList(utf8.encode(toQueryString(payload)));\n\t\t} else \
		 {\n\t\t\theaders['Content-Type'] = 'application/json';\n\t\t\t// Absent optional \
		 arguments are omitted, as `serde_qs` omits them.\n\t\t\tfinal present = <String, \
		 Object?>{\n\t\t\t\tfor (final entry in payload.entries)\n\t\t\t\t\tif (entry.value != \
		 null) entry.key: entry.value,\n\t\t\t};\n\t\t\tbody = \
		 Uint8List.fromList(utf8.encode(jsonEncode(present)));\n\t\t}\n\t\tlate http.Response \
		 response;\n\t\ttry {\n\t\t\tfinal uri = Uri.parse(url);\n\t\t\tfinal request = \
		 http.Request(method, uri)\n\t\t\t\t..headers.addAll(headers);\n\t\t\tif (body != null) \
		 request.bodyBytes = body;\n\t\t\tresponse = await http.Response.fromStream(await \
		 _http.send(request));\n\t\t} catch (error) {\n\t\t\tthrow FnError('network', 'could not \
		 reach $url: $error');\n\t\t}\n\t\tif (response.statusCode >= 400) {\n\t\t\tthrow \
		 FnError.fromResponse(response.statusCode, response.body);\n\t\t}\n\t\tif \
		 (response.body.isEmpty) return null;\n\t\treturn jsonDecode(response.body);\n\t}\n}\n",
	);
	out
}

fn pubspec(context: &Context<'_>, package: &str) -> String {
	format!(
		"{banner}name: {package}\ndescription: A typed client for the {title} RPC.\nversion: \
		 {version}\npublish_to: none\n\nenvironment:\n  sdk: '>=3.0.0 <4.0.0'\n\ndependencies:\n  \
		 http: ^1.2.0\n",
		banner = context.banner("#"),
		title = context.spec.title,
		version = &context.version,
	)
}

fn readme(context: &Context<'_>) -> String {
	let example = context
		.functions()
		.next()
		.map_or_else(String::new, |function| {
			let args: Vec<String> = function
				.params
				.iter()
				.filter(|param| param.is_required())
				.map(|param| format!("{}: ...", dart_identifier(&param.name)))
				.collect();
			format!(
				"final result = await client.{}({});\n",
				dart_identifier(&function.name),
				args.join(", ")
			)
		});
	let mut skipped = String::new();
	if !context.skipped.is_empty() {
		skipped.push_str("\n## Not generated\n\n");
		for (name, reason) in &context.skipped {
			skipped.push_str(&format!("- `{name}`: {reason}\n"));
		}
	}
	format!(
		"# {name}

A typed Dart client for the **{title} {version}** RPC, generated by `leptoskit fns generate \
		 --target dart`.

It depends on `package:http`, and the calls go to the same routes the app itself uses.

## Usage

```dart
import 'package:{package}/{package}.dart';

final client = Client(serverUrl: {server_url});

try {{
  {example}}} on FnError catch (error) {{
  if (error.retryable) {{
    // a network or server failure: safe to try again
  }}
  rethrow;
}}
```

`FnError.kind` is one of `network`, `invalid`, `server`, `decode`, or `other`, mirroring the \
		 server's classification; `retryable` is true for `network` and `server`. Types that Dart \
		 cannot model yet (tagged unions) appear as `Object?` holding the raw JSON.{skipped}",
		name = context.options.package_name,
		package = dart_package_name(&context.options.package_name),
		title = context.spec.title,
		version = context.spec.version,
		server_url = dart_string(
			context
				.server_url
				.as_deref()
				.unwrap_or("https://example.com")
		),
	)
}

/// The error type, `serde_qs` encoder, and helpers every package shares.
const RUNTIME: &str = r"/// How a failed call is classified, mirroring `leptoskit_fns::FnError`.
class FnError implements Exception {
  /// One of `network`, `invalid`, `server`, `decode`, or `other`.
  final String kind;

  /// The human-readable message.
  final String message;

  /// The HTTP status, when a response arrived.
  final int? status;

  /// The response body, when a response arrived.
  final String? raw;

  FnError(this.kind, this.message, {this.status, this.raw});

  /// Whether retrying the call could plausibly succeed.
  bool get retryable => kind == 'network' || kind == 'server';

  /// Parses leptos's error body: `Kind|message` as `text/plain`.
  factory FnError.fromResponse(int status, String body) {
    const kinds = <String, String>{
      'Request': 'network',
      'Args': 'invalid',
      'MissingArg': 'invalid',
      'ServerError': 'server',
      'MiddlewareError': 'server',
      'WrappedServerFn': 'server',
      'Deserialization': 'decode',
      'Serialization': 'decode',
    };
    final separator = body.indexOf('|');
    if (separator == -1) {
      final text = body.isEmpty ? 'HTTP $status' : body;
      return FnError('other', text, status: status, raw: body);
    }
    final variant = body.substring(0, separator);
    return FnError(
      kinds[variant] ?? 'other',
      body.substring(separator + 1),
      status: status,
      raw: body,
    );
  }

  @override
  String toString() => 'FnError($kind): $message';
}

/// Encodes a value the way `serde_qs` does: nested keys in brackets, lists by
/// index, `null` omitted.
String toQueryString(Map<String, Object?> value) {
  final pairs = <String>[];

  void append(String key, Object? item) {
    if (item == null) return;
    if (item is bool) {
      pairs.add('$key=${item ? 'true' : 'false'}');
    } else if (item is List) {
      for (var index = 0; index < item.length; index++) {
        append('$key[$index]', item[index]);
      }
    } else if (item is Map<String, Object?>) {
      for (final entry in item.entries) {
        append('$key[${entry.key}]', entry.value);
      }
    } else {
      pairs.add('$key=${Uri.encodeQueryComponent('$item')}');
    }
  }

  for (final entry in value.entries) {
    append(entry.key, entry.value);
  }
  return pairs.join('&');
}

";
