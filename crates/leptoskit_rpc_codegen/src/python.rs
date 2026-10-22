//! The Python client: a package with `TypedDict` models and standard-library
//! transport, plus an `httpx`-backed async client.
//!
//! Models are `TypedDict`s rather than classes so values stay plain `dict`s
//! that round-trip through `json` unchanged — the generated code never has to
//! guess how to serialize a user-defined class. Type aliases are written as
//! quoted `TypeAlias` strings, which type checkers understand and which free
//! the module from declaration-order constraints.

// Generators build source text; the allocation `write!` would save is noise here.
#![allow(clippy::format_push_string)]

use std::collections::HashSet;

use serde_json::Value;

use crate::generate::Context;
use crate::generate::Generated;
use crate::generate::GeneratedFile;
use crate::naming::py_identifier;
use crate::naming::py_package_name;
use crate::naming::py_string;
use crate::naming::type_name;
use crate::schema::ObjectShape;
use crate::schema::Shape;
use crate::schema::analyze;
use crate::schema::description;
use crate::schema::has_default;
use crate::spec::ApiFunction;
use crate::spec::ApiSpec;
use crate::spec::HttpInput;

pub(crate) fn generate(context: &Context<'_>) -> Generated {
	let module = py_package_name(&context.options.package_name);
	let mut types = PyTypes::default();
	let models = models_module(context, &mut types);
	let client = client_module(context, &mut types);
	Generated {
		files: vec![
			GeneratedFile::scaffold("pyproject.toml", pyproject(context, &module)),
			GeneratedFile::code(
				format!("{module}/__init__.py"),
				init_module(context, &types),
			),
			GeneratedFile::code(format!("{module}/models.py"), models),
			GeneratedFile::code(format!("{module}/client.py"), client),
			GeneratedFile::scaffold("README.md", readme(context, &module)),
		],
	}
}

/// Tracks the model names in declaration order, plus synthesized `TypedDict`s
/// for anonymous object schemas.
#[derive(Default)]
struct PyTypes {
	/// Every public model name, for `__all__`.
	names: Vec<String>,
	taken: HashSet<String>,
	/// Synthesized `TypedDict` definitions, emitted before the aliases.
	synthesized: Vec<String>,
}

impl PyTypes {
	fn reserve(&mut self, wanted: &str) -> String {
		let mut name = wanted.to_owned();
		let mut counter = 2;
		while !self.taken.insert(name.clone()) {
			name = format!("{wanted}{counter}");
			counter += 1;
		}
		self.names.push(name.clone());
		name
	}

	/// A Python type expression for `schema`, synthesizing a `TypedDict` named
	/// after `hint` when it is an anonymous object.
	fn render(&mut self, schema: &Value, hint: &str, spec: &ApiSpec) -> String {
		let analyzed = analyze(schema);
		let base = match analyzed.shape {
			Shape::Ref(name) => {
				match spec.schema(name) {
					Some(_) => type_name(name),
					None => "Any".to_owned(),
				}
			}
			Shape::Null => "None".to_owned(),
			Shape::Boolean => "bool".to_owned(),
			Shape::Integer { .. } => "int".to_owned(),
			Shape::Number { .. } => "float".to_owned(),
			Shape::String {
				values: Some(values),
				..
			} => {
				format!(
					"Literal[{}]",
					values
						.iter()
						.map(|value| py_string(value))
						.collect::<Vec<_>>()
						.join(", ")
				)
			}
			Shape::String {
				constant: Some(constant),
				..
			} => format!("Literal[{}]", py_string(constant)),
			Shape::String { .. } => "str".to_owned(),
			Shape::Array { items } => {
				format!(
					"list[{}]",
					items.map_or_else(
						|| "Any".to_owned(),
						|items| self.render(items, &format!("{hint}Item"), spec)
					)
				)
			}
			Shape::Tuple(items) => {
				let parts: Vec<String> = items
					.iter()
					.enumerate()
					.map(|(index, item)| self.render(item, &format!("{hint}{index}"), spec))
					.collect();
				format!("tuple[{}]", parts.join(", "))
			}
			Shape::Object(object) => {
				if object.properties.is_empty() {
					match object.additional {
						Some(values) => {
							format!(
								"dict[str, {}]",
								self.render(values, &format!("{hint}Value"), spec)
							)
						}
						None => "dict[str, Any]".to_owned(),
					}
				} else {
					let name = self.reserve(&type_name(hint));
					let definition = typed_dict(&name, None, &object, self, spec, &name);
					self.synthesized.push(definition);
					name
				}
			}
			Shape::Map { values } => {
				format!(
					"dict[str, {}]",
					values.map_or_else(
						|| "Any".to_owned(),
						|values| self.render(values, &format!("{hint}Value"), spec)
					)
				)
			}
			Shape::Union(variants) => {
				let parts: Vec<String> = variants
					.iter()
					.enumerate()
					.map(|(index, variant)| {
						let variant_hint = variant_hint(variant).map_or_else(
							|| format!("{hint}Variant{}", index + 1),
							|tag| format!("{hint}{}", type_name(tag)),
						);
						self.render(variant, &variant_hint, spec)
					})
					.collect();
				format!("Union[{}]", parts.join(", "))
			}
			Shape::Any => "Any".to_owned(),
		};
		if analyzed.nullable && base != "None" {
			format!("Optional[{base}]")
		} else {
			base
		}
	}
}

/// A discriminator value to name an enum variant's synthesized type after.
fn variant_hint(variant: &Value) -> Option<&str> {
	let object = variant.as_object()?;
	let properties = object.get("properties")?.as_object()?;
	properties.values().find_map(|property| {
		property
			.as_object()
			.and_then(|property| property.get("const"))
			.and_then(Value::as_str)
	})
}

/// A `TypedDict` definition. Class syntax when every key is a plain
/// identifier; functional syntax otherwise.
fn typed_dict(
	name: &str,
	docs: Option<&str>,
	object: &ObjectShape<'_>,
	types: &mut PyTypes,
	spec: &ApiSpec,
	hint: &str,
) -> String {
	let mut fields: Vec<(String, String, bool, Option<String>)> = Vec::new();
	for (property, schema) in &object.properties {
		let required = object.is_required(property) && !has_default(schema);
		let rendered = types.render(schema, &format!("{hint}{}", type_name(property)), spec);
		fields.push((
			(*property).to_owned(),
			rendered,
			required,
			description(schema).map(ToOwned::to_owned),
		));
	}
	let class_syntax = fields.iter().all(|(key, ..)| py_is_plain_key(key));
	let mut out = String::new();
	if class_syntax {
		out.push_str(&format!("class {name}(TypedDict, total=False):\n"));
		if let Some(docs) = docs {
			out.push_str(&docstring(docs, "\t"));
		}
		if fields.is_empty() {
			out.push_str("\tpass\n");
		}
		for (key, rendered, required, field_docs) in &fields {
			if let Some(field_docs) = field_docs {
				for line in field_docs.lines() {
					out.push_str(&format!("\t# {line}\n"));
				}
			}
			let annotation = if *required {
				format!("Required[{rendered}]")
			} else {
				format!("NotRequired[{rendered}]")
			};
			out.push_str(&format!("\t{key}: {annotation}\n"));
		}
	} else {
		out.push_str(&format!(
			"{name} = TypedDict(\n\t{},\n\t{{\n",
			py_string(name)
		));
		for (key, rendered, required, field_docs) in &fields {
			if let Some(field_docs) = field_docs {
				for line in field_docs.lines() {
					out.push_str(&format!("\t\t# {line}\n"));
				}
			}
			let annotation = if *required {
				format!("Required[{}]", py_string(rendered))
			} else {
				format!("NotRequired[{}]", py_string(rendered))
			};
			out.push_str(&format!("\t\t{}: {annotation},\n", py_string(key)));
		}
		out.push_str("\t},\n\ttotal=False,\n)\n");
		if let Some(docs) = docs {
			out.push_str(&format!("{name}.__doc__ = {}\n", py_string(docs)));
		}
	}
	out.push('\n');
	out
}

/// Whether a wire name can be a `class`-syntax `TypedDict` key.
fn py_is_plain_key(name: &str) -> bool {
	let mut characters = name.chars();
	characters
		.next()
		.is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
		&& characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn docstring(docs: &str, indent: &str) -> String {
	let safe = docs.replace("\"\"\"", "\\\"\\\"\\\"");
	let mut lines = safe.lines();
	let first = lines.next().unwrap_or_default();
	let rest: Vec<&str> = lines.collect();
	if rest.is_empty() {
		return format!("{indent}\"\"\"{first}\"\"\"\n");
	}
	let mut out = format!("{indent}\"\"\"{first}\n");
	for line in rest {
		if line.is_empty() {
			out.push('\n');
		} else {
			out.push_str(&format!("{indent}{line}\n"));
		}
	}
	out.push_str(&format!("{indent}\"\"\"\n"));
	out
}

fn models_module(context: &Context<'_>, types: &mut PyTypes) -> String {
	let mut definitions = String::new();
	let mut aliases = String::new();
	for (schema_name, schema) in &context.spec.schemas {
		let name = types.reserve(&type_name(schema_name));
		let analyzed = analyze(schema);
		match (&analyzed.shape, analyzed.nullable) {
			(Shape::Object(object), false) if !object.properties.is_empty() => {
				definitions.push_str(&typed_dict(
					&name,
					description(schema),
					object,
					types,
					context.spec,
					&name,
				));
			}
			_ => {
				let rendered = types.render(schema, &name, context.spec);
				if let Some(docs) = description(schema) {
					for line in docs.lines() {
						aliases.push_str(&format!("# {line}\n"));
					}
				}
				aliases.push_str(&format!("{name}: TypeAlias = {}\n\n", py_string(&rendered)));
			}
		}
	}
	let mut out = format!(
		"{}\"\"\"Types for the {title} {version} RPC.\"\"\"\n\nfrom __future__ import \
		 annotations\n\nfrom typing import Any, Literal, NotRequired, Optional, Required, \
		 TypeAlias, TypedDict, Union\n\n",
		context.banner("#"),
		title = context.spec.title,
		version = context.spec.version
	);
	// Synthesized dicts come first: class bodies only hold lazy annotations,
	// so order among them does not matter, while aliases are quoted.
	for synthesized in &types.synthesized {
		out.push_str(synthesized);
	}
	out.push_str(&definitions);
	out.push_str(&aliases);
	out
}

/// The keyword-only parameters of a method, with wire-name mapping.
struct Signature {
	parameters: String,
	payload: String,
}

fn signature(function: &ApiFunction, types: &mut PyTypes, spec: &ApiSpec) -> Signature {
	if function.params.is_empty() {
		return Signature {
			parameters: String::new(),
			payload: "{}".to_owned(),
		};
	}
	let mut parameters = vec!["*".to_owned()];
	let mut payload = Vec::new();
	let hint = type_name(&function.name);
	for param in &function.params {
		let identifier = py_identifier(&param.name);
		let rendered = types.render(
			&param.schema,
			&format!("{hint}{}", type_name(&param.name)),
			spec,
		);
		if param.is_required() {
			parameters.push(format!("{identifier}: {rendered}"));
		} else {
			let rendered = if rendered.starts_with("Optional[") {
				rendered
			} else {
				format!("Optional[{rendered}]")
			};
			parameters.push(format!("{identifier}: {rendered} = None"));
		}
		payload.push(format!("{}: {identifier}", py_string(&param.name)));
	}
	Signature {
		parameters: parameters.join(", "),
		payload: format!("{{{}}}", payload.join(", ")),
	}
}

fn client_module(context: &Context<'_>, types: &mut PyTypes) -> String {
	let mut methods = String::new();
	let mut async_methods = String::new();
	for function in context.functions() {
		let Signature {
			parameters,
			payload,
		} = signature(function, types, context.spec);
		let output = types.render(
			&function.output_schema,
			&format!("{}Output", type_name(&function.name)),
			context.spec,
		);
		let name = py_identifier(&function.name);
		let encoding = match function.input {
			HttpInput::Query => "query",
			HttpInput::Form => "form",
			HttpInput::Json | HttpInput::Other { .. } => "json",
		};
		let mut docs = function.docs.clone();
		let documented: Vec<&crate::spec::ApiParam> = function
			.params
			.iter()
			.filter(|param| !param.docs.is_empty())
			.collect();
		if !documented.is_empty() {
			docs.push_str("\n\nArguments:");
			for param in documented {
				docs.push_str(&format!(
					"\n\t{}: {}",
					py_identifier(&param.name),
					param.docs.trim()
				));
			}
		}
		if function.deprecated {
			docs.push_str("\n\nDeprecated.");
		}
		let docs = if docs.is_empty() {
			String::new()
		} else {
			docstring(&docs, "\t\t")
		};
		let separator = if parameters.is_empty() { "" } else { ", " };
		methods.push_str(&format!(
			"\n\tdef {name}(self{separator}{parameters}) -> {output}:\n{docs}\t\treturn \
			 self._call({}, {}, {}, {payload})\n",
			py_string(&function.method),
			py_string(&function.path),
			py_string(encoding),
		));
		async_methods.push_str(&format!(
			"\n\tasync def {name}(self{separator}{parameters}) -> {output}:\n{docs}\t\treturn \
			 await self._call({}, {}, {}, {payload})\n",
			py_string(&function.method),
			py_string(&function.path),
			py_string(encoding),
		));
	}
	let imports = if types.names.is_empty() {
		String::new()
	} else {
		format!("from .models import {}\n", types.names.join(", "))
	};
	format!(
		"{banner}\"\"\"A typed client for the {title} {version} RPC.\"\"\"\n\nfrom __future__ \
		 import annotations\n\nimport json\nimport urllib.error\nimport urllib.parse\nimport \
		 urllib.request\nfrom typing import Any, Mapping, \
		 Optional\n\n{imports}\nDEFAULT_SERVER_URL = {base_url}\n\"\"\"The server origin baked in \
		 at generation time; empty means same origin.\"\"\"\n\n{runtime}\nclass \
		 Client(_BaseClient):\n\t\"\"\"A synchronous client over the standard library.\n\n\tPass \
		 ``server_url`` to point it at a server, ``headers`` for authorization, \
		 and\n\t``timeout`` in seconds.\n\t\"\"\"\n\n\tdef _call(self, method: str, path: str, \
		 encoding: str, payload: Mapping[str, Any]) -> Any:\n\t\turl, headers, data = \
		 self._prepare(method, path, encoding, payload)\n\t\trequest = \
		 urllib.request.Request(url, data=data, method=method, \
		 headers=headers)\n\t\ttry:\n\t\t\twith urllib.request.urlopen(request, \
		 timeout=self.timeout) as response:\n\t\t\t\tstatus = response.status\n\t\t\t\tbody = \
		 response.read().decode(\"utf-8\")\n\t\texcept urllib.error.HTTPError as \
		 error:\n\t\t\tbody = error.read().decode(\"utf-8\", \"replace\")\n\t\t\traise \
		 FnError.from_response(error.code, body) from error\n\t\texcept urllib.error.URLError as \
		 error:\n\t\t\traise FnError(\"network\", f\"could not reach {{url}}: {{error.reason}}\") \
		 from error\n\t\treturn _decode(status, body)\n{methods}\n\nclass \
		 AsyncClient(_BaseClient):\n\t\"\"\"An asynchronous client over ``httpx`` (install the \
		 ``async`` extra).\n\n\tPass an existing ``httpx.AsyncClient`` as ``http`` to share \
		 connections.\n\t\"\"\"\n\n\tdef __init__(\n\t\tself,\n\t\tbase_url: str = \
		 DEFAULT_SERVER_URL,\n\t\t*,\n\t\theaders: Optional[Mapping[str, str]] = \
		 None,\n\t\ttimeout: float = 30.0,\n\t\thttp: Any = None,\n\t) -> \
		 None:\n\t\tsuper().__init__(base_url, headers=headers, timeout=timeout)\n\t\tif http is \
		 None:\n\t\t\ttry:\n\t\t\t\timport httpx\n\t\t\texcept ImportError as error:  # pragma: \
		 no cover\n\t\t\t\traise ImportError(\n\t\t\t\t\t\"AsyncClient needs httpx; install the \
		 async extra or pass http=\"\n\t\t\t\t) from error\n\t\t\thttp = \
		 httpx.AsyncClient(timeout=timeout)\n\t\tself.http = http\n\n\tasync def aclose(self) -> \
		 None:\n\t\t\"\"\"Closes the underlying ``httpx`` client.\"\"\"\n\t\tawait \
		 self.http.aclose()\n\n\tasync def __aenter__(self) -> \"AsyncClient\":\n\t\treturn \
		 self\n\n\tasync def __aexit__(self, *exc_info: object) -> None:\n\t\tawait \
		 self.aclose()\n\n\tasync def _call(self, method: str, path: str, encoding: str, payload: \
		 Mapping[str, Any]) -> Any:\n\t\turl, headers, data = self._prepare(method, path, \
		 encoding, payload)\n\t\ttry:\n\t\t\tresponse = await self.http.request(method, url, \
		 headers=headers, content=data)\n\t\texcept Exception as error:  # httpx.TransportError \
		 and friends\n\t\t\traise FnError(\"network\", f\"could not reach {{url}}: {{error}}\") \
		 from error\n\t\tif response.status_code >= 400:\n\t\t\traise \
		 FnError.from_response(response.status_code, response.text)\n\t\treturn \
		 _decode(response.status_code, response.text)\n{async_methods}",
		banner = context.banner("#"),
		title = context.spec.title,
		version = context.spec.version,
		base_url = py_string(&context.server_url.clone().unwrap_or_default()),
		runtime = RUNTIME,
	)
}

fn init_module(context: &Context<'_>, types: &PyTypes) -> String {
	let mut exports: Vec<String> = vec![
		"AsyncClient".to_owned(),
		"Client".to_owned(),
		"DEFAULT_SERVER_URL".to_owned(),
		"FnError".to_owned(),
		"to_query_string".to_owned(),
	];
	exports.extend(types.names.iter().cloned());
	let quoted: Vec<String> = exports.iter().map(|name| py_string(name)).collect();
	let model_imports = if types.names.is_empty() {
		String::new()
	} else {
		format!("from .models import {}\n", types.names.join(", "))
	};
	format!(
		"{}\"\"\"A typed client for the {} {} RPC.\"\"\"\n\nfrom .client import AsyncClient, \
		 Client, DEFAULT_SERVER_URL, FnError, to_query_string\n{model_imports}\n__all__ = [{}]\n",
		context.banner("#"),
		context.spec.title,
		context.spec.version,
		quoted.join(", ")
	)
}

fn pyproject(context: &Context<'_>, module: &str) -> String {
	format!(
		"{banner}[build-system]\nrequires = [\"hatchling\"]\nbuild-backend = \
		 \"hatchling.build\"\n\n[project]\nname = {name}\nversion = {version}\ndescription = \"A \
		 typed client for the {title} RPC.\"\nreadme = \"README.md\"\nrequires-python = \
		 \">=3.11\"\ndependencies = []\n\n[project.optional-dependencies]\nasync = \
		 [\"httpx>=0.27\"]\n\n[tool.hatch.build.targets.wheel]\npackages = [{module}]\n",
		banner = context.banner("#"),
		name = py_string(&context.options.package_name),
		version = py_string(&context.version),
		title = context.spec.title,
		module = py_string(module),
	)
}

fn readme(context: &Context<'_>, module: &str) -> String {
	let example = context
		.functions()
		.next()
		.map_or_else(String::new, |function| {
			let args: Vec<String> = function
				.params
				.iter()
				.filter(|param| param.is_required())
				.map(|param| format!("{}=...", py_identifier(&param.name)))
				.collect();
			format!(
				"result = api.{}({})\n",
				py_identifier(&function.name),
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

A typed Python client for the **{title} {version}** RPC, generated by `leptoskit rpc generate \
		 --target python`.

The synchronous `Client` uses only the standard library; `AsyncClient` needs the `async` extra \
		 (`pip install '{name}[async]'`), which brings in `httpx`. Models are `TypedDict`s, so \
		 values are plain dictionaries, and the calls go to the same routes the app itself uses.

## Usage

```python
from {module} import Client, FnError

api = Client({base_url})

try:
    {example}except FnError as error:
    if error.retryable:
        ...  # a network or server failure: safe to try again
    raise
```

```python
from {module} import AsyncClient

async with AsyncClient({base_url}) as api:
    {example}```

`FnError.kind` is one of `network`, `invalid`, `server`, `decode`, or `other`, mirroring the \
		 server's classification; `retryable` is true for `network` and `server`.{skipped}",
		name = context.options.package_name,
		title = context.spec.title,
		version = context.spec.version,
		base_url = py_string(
			context
				.server_url
				.as_deref()
				.unwrap_or("https://example.com")
		),
	)
}

/// The transport, error type, and `serde_qs` encoder every client shares.
const RUNTIME: &str = r#"
class FnError(Exception):
    """A failed RPC call, classified like ``leptoskit_rpc::FnError``.

    ``kind`` is one of ``network``, ``invalid``, ``server``, ``decode``, or ``other``.
    """

    def __init__(
        self,
        kind: str,
        message: str,
        *,
        status: Optional[int] = None,
        raw: Optional[str] = None,
    ) -> None:
        super().__init__(message)
        self.kind = kind
        self.message = message
        self.status = status
        self.raw = raw

    @property
    def retryable(self) -> bool:
        """Whether retrying the call could plausibly succeed."""
        return self.kind in ("network", "server")

    @classmethod
    def from_response(cls, status: int, body: str) -> "FnError":
        """Parses leptos's error body: ``Kind|message`` as ``text/plain``."""
        variant, separator, message = body.partition("|")
        if not separator:
            return cls("other", body or f"HTTP {status}", status=status, raw=body)
        kind = _ERROR_KINDS.get(variant, "other")
        return cls(kind, message, status=status, raw=body)


_ERROR_KINDS = {
    "Request": "network",
    "Args": "invalid",
    "MissingArg": "invalid",
    "ServerError": "server",
    "MiddlewareError": "server",
    "WrappedServerFn": "server",
    "Deserialization": "decode",
    "Serialization": "decode",
}


def to_query_string(value: Mapping[str, Any]) -> str:
    """Encodes a value the way ``serde_qs`` does.

    Nested keys go in brackets, lists by index, and ``None`` is omitted.
    """
    pairs: list[tuple[str, str]] = []

    def append(key: str, item: Any) -> None:
        if item is None:
            return
        if isinstance(item, bool):
            pairs.append((key, "true" if item else "false"))
        elif isinstance(item, (list, tuple)):
            for index, entry in enumerate(item):
                append(f"{key}[{index}]", entry)
        elif isinstance(item, Mapping):
            for field, entry in item.items():
                append(f"{key}[{field}]", entry)
        else:
            pairs.append((key, str(item)))

    for key, item in value.items():
        append(key, item)
    return urllib.parse.urlencode(pairs)


def _decode(status: int, body: str) -> Any:
    if body == "":
        return None
    try:
        return json.loads(body)
    except ValueError as error:
        raise FnError("decode", f"the response was not JSON: {error}", status=status, raw=body) from error


class _BaseClient:
    def __init__(
        self,
        base_url: str = DEFAULT_SERVER_URL,
        *,
        headers: Optional[Mapping[str, str]] = None,
        timeout: float = 30.0,
    ) -> None:
        self.base_url = base_url.rstrip("/")
        self.headers = dict(headers or {})
        self.timeout = timeout

    def _prepare(
        self, method: str, path: str, encoding: str, payload: Mapping[str, Any]
    ) -> tuple[str, dict[str, str], Optional[bytes]]:
        url = f"{self.base_url}{path}"
        headers = {"Accept": "application/json", **self.headers}
        data: Optional[bytes] = None
        if encoding == "query":
            query = to_query_string(payload)
            if query:
                url += ("&" if "?" in url else "?") + query
        elif encoding == "form":
            headers["Content-Type"] = "application/x-www-form-urlencoded"
            data = to_query_string(payload).encode("utf-8")
        else:
            headers["Content-Type"] = "application/json"
            # Absent optional arguments are omitted, as `serde_qs` omits them.
            present = {key: item for key, item in payload.items() if item is not None}
            data = json.dumps(present).encode("utf-8")
        return url, headers, data

"#;
