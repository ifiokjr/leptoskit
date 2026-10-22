//! The Go client: one package over `net/http`.
//!
//! Models are plain structs with `json` tags (so `encoding/json` round-trips
//! them unchanged), string enums become named string types with constants,
//! and anything Go cannot model yet (tagged unions) falls back to `any`
//! holding the decoded JSON. The wire encoding — `serde_qs` brackets for form
//! and query, JSON bodies — mirrors what the server's codecs read.

// Generators build source text; the allocation `write!` would save is noise here.
#![allow(clippy::format_push_string)]

use std::collections::HashSet;

use serde_json::Value;

use crate::generate::Context;
use crate::generate::Generated;
use crate::generate::GeneratedFile;
use crate::naming::go_package_name;
use crate::naming::go_string;
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
	let package = go_package_name(&context.options.package_name);
	let mut types = GoTypes::default();
	let models = models_file(context, &mut types);
	let client = client_file(context, &package, &mut types);
	Generated {
		files: vec![
			GeneratedFile::scaffold("go.mod", go_mod(context)),
			GeneratedFile::code("models.go", models),
			GeneratedFile::code("client.go", client),
			GeneratedFile::scaffold("README.md", readme(context, &package)),
		],
	}
}

/// Tracks model names and the synthesized declarations for anonymous shapes.
#[derive(Default)]
struct GoTypes {
	names: Vec<String>,
	taken: HashSet<String>,
	synthesized: Vec<String>,
}

impl GoTypes {
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

	/// A Go type expression for `schema`, synthesizing structs for anonymous
	/// objects and named types for inline enums, named after `hint`.
	fn render(&mut self, schema: &Value, hint: &str, spec: &ApiSpec) -> String {
		let analyzed = analyze(schema);
		let base = match analyzed.shape {
			Shape::Ref(name) => {
				match spec.schema(name) {
					Some(_) => type_name(name),
					None => "any".to_owned(),
				}
			}
			Shape::Null => "struct{}".to_owned(),
			Shape::Boolean => "bool".to_owned(),
			Shape::Integer { format } => integer_type(format),
			Shape::Number { .. } => "float64".to_owned(),
			Shape::String {
				values: Some(_), ..
			} => {
				let name = self.reserve(&type_name(hint));
				self.synthesized.push(enum_declaration(schema, &name));
				name
			}
			Shape::String { .. } => "string".to_owned(),
			Shape::Array { items } => {
				let inner = items.map_or_else(
					|| "any".to_owned(),
					|items| self.render(items, &format!("{hint}Item"), spec),
				);
				// Slices and maps are nilable; Option only needs a pointer for
				// value types.
				format!("[]{inner}")
			}
			Shape::Tuple(_) => "[]any".to_owned(),
			Shape::Object(object) => {
				if object.properties.is_empty() {
					match object.additional {
						Some(values) => {
							format!(
								"map[string]{}",
								self.render(values, &format!("{hint}Value"), spec)
							)
						}
						None => "map[string]any".to_owned(),
					}
				} else {
					let name = self.reserve(&type_name(hint));
					let declaration =
						struct_declaration(&name, description(schema), &object, self, spec, &name);
					self.synthesized.push(declaration);
					name
				}
			}
			Shape::Map { values } => {
				format!(
					"map[string]{}",
					values.map_or_else(
						|| "any".to_owned(),
						|values| self.render(values, &format!("{hint}Value"), spec)
					)
				)
			}
			Shape::Union(_) | Shape::Any => "any".to_owned(),
		};
		if analyzed.nullable && needs_pointer(&base) {
			format!("*{base}")
		} else {
			base
		}
	}
}

/// Go type for an integer schema format.
pub(crate) fn integer_type(format: Option<&str>) -> String {
	match format {
		Some("int8") => "int8".to_owned(),
		Some("int16") => "int16".to_owned(),
		Some("int32") => "int32".to_owned(),
		Some("uint8") => "uint8".to_owned(),
		Some("uint16") => "uint16".to_owned(),
		Some("uint32") => "uint32".to_owned(),
		Some("uint64") => "uint64".to_owned(),
		_ => "int64".to_owned(),
	}
}

/// Whether an optional value of this type needs a pointer to express absence:
/// slices, maps, `any`, and structs (via a null in JSON) are nilable or
/// nullable already.
fn needs_pointer(type_name: &str) -> bool {
	!(type_name.starts_with("[]")
		|| type_name.starts_with("map[")
		|| type_name == "any"
		|| type_name == "struct{}")
}

/// One field of a generated struct.
struct Field {
	go_name: String,
	type_name: String,
	tag: String,
	docs: Vec<String>,
	/// `omitempty` for optional fields.
	optional: bool,
}

/// A struct declaration, with gofmt-style tag alignment.
fn struct_declaration(
	name: &str,
	docs: Option<&str>,
	object: &ObjectShape<'_>,
	types: &mut GoTypes,
	spec: &ApiSpec,
	hint: &str,
) -> String {
	let mut fields = Vec::new();
	for (wire, schema) in &object.properties {
		let field_type = types.render(schema, &format!("{hint}{}", type_name(wire)), spec);
		let optional = !object.is_required(wire) || has_default(schema);
		fields.push(Field {
			go_name: type_name(wire),
			type_name: field_type,
			tag: (*wire).to_owned(),
			docs: description(schema)
				.map(|docs| docs.lines().map(str::trim).map(str::to_owned).collect())
				.unwrap_or_default(),
			optional,
		});
	}

	let mut out = String::new();
	for line in docs.unwrap_or_default().lines() {
		out.push_str(&format!("// {line}\n"));
	}
	out.push_str(&format!("type {name} struct {{\n"));
	let width = fields
		.iter()
		.map(|field| field.go_name.len())
		.max()
		.unwrap_or_default();
	let type_width = fields
		.iter()
		.map(|field| field.type_name.len())
		.max()
		.unwrap_or_default();
	for field in &fields {
		for line in &field.docs {
			out.push_str(&format!("\t// {line}\n"));
		}
		let omit = if field.optional { ",omitempty" } else { "" };
		out.push_str(&format!(
			"\t{:width$} {:type_width$} `json:\"{}{omit}\"`\n",
			field.go_name, field.type_name, field.tag,
		));
	}
	out.push_str("}\n\n");
	out
}

/// A named string type with constants, for a string-enum schema.
fn enum_declaration(schema: &Value, name: &str) -> String {
	let values: Vec<&str> = match analyze(schema).shape {
		Shape::String {
			values: Some(values),
			..
		} => values,
		_ => Vec::new(),
	};
	let mut out = String::new();
	if let Some(docs) = description(schema) {
		for line in docs.lines() {
			out.push_str(&format!("// {line}\n"));
		}
	}
	out.push_str(&format!("type {name} string\n\nconst (\n"));
	let mut seen: HashSet<String> = HashSet::new();
	let mut constants = Vec::new();
	for value in &values {
		let mut constant = format!("{name}{}", type_name(value));
		// Values differing only in case still need distinct constant names.
		let mut counter = 2;
		while !seen.insert(constant.clone()) {
			constant = format!("{name}{}{counter}", type_name(value));
			counter += 1;
		}
		constants.push((constant, (*value).to_owned()));
	}
	let width = constants
		.iter()
		.map(|(constant, _)| constant.len())
		.max()
		.unwrap_or_default();
	for (constant, value) in &constants {
		out.push_str(&format!(
			"\t{:width$} {name} = {}\n",
			constant,
			go_string(value)
		));
	}
	out.push_str(")\n\n");
	out
}

fn models_file(context: &Context<'_>, types: &mut GoTypes) -> String {
	let mut declarations = String::new();
	let mut aliases = String::new();
	for (schema_name, schema) in &context.spec.schemas {
		let name = types.reserve(&type_name(schema_name));
		let analyzed = analyze(schema);
		match (&analyzed.shape, analyzed.nullable) {
			(Shape::Object(object), false) if !object.properties.is_empty() => {
				declarations.push_str(&struct_declaration(
					&name,
					description(schema),
					object,
					types,
					context.spec,
					&name,
				));
			}
			(
				Shape::String {
					values: Some(_), ..
				},
				_,
			) => {
				declarations.push_str(&enum_declaration(schema, &name));
			}
			_ => {
				let rendered = types.render(schema, &name, context.spec);
				let rendered = if analyzed.nullable && needs_pointer(&rendered) {
					format!("*{rendered}")
				} else {
					rendered
				};
				if let Some(docs) = description(schema) {
					for line in docs.lines() {
						aliases.push_str(&format!("// {line}\n"));
					}
				}
				aliases.push_str(&format!("type {name} = {rendered}\n\n"));
			}
		}
	}
	let mut out = context.banner("//");
	out.push_str(&format!(
		"// Code generated by leptoskit; DO NOT EDIT.\n\npackage {package}\n\n// Types for the \
		 {title} {version} RPC.\n\n",
		package = go_package_name(&context.options.package_name),
		title = context.spec.title,
		version = context.spec.version
	));
	for synthesized in &types.synthesized {
		out.push_str(synthesized);
	}
	out.push_str(&declarations);
	out.push_str(&aliases);
	out
}

/// The signature and body payload builder for one method.
struct Method {
	name: String,
	parameters: Vec<String>,
	encoding: &'static str,
	payload: String,
	returns: String,
}

fn method(function: &ApiFunction, types: &mut GoTypes, spec: &ApiSpec) -> Method {
	let hint = type_name(&function.name);
	let mut parameters = vec!["ctx context.Context".to_owned()];
	let mut fields: Vec<Field> = Vec::new();
	for param in &function.params {
		let param_type = types.render(
			&param.schema,
			&format!("{hint}{}", type_name(&param.name)),
			spec,
		);
		let identifier = crate::naming::go_identifier(&param.name);
		parameters.push(format!("{identifier} {param_type}"));
		let optional = !param.is_required();
		fields.push(Field {
			go_name: type_name(&param.name),
			type_name: param_type,
			tag: param.name.clone(),
			docs: Vec::new(),
			optional,
		});
	}
	let width = fields
		.iter()
		.map(|field| field.go_name.len())
		.max()
		.unwrap_or_default();
	let type_width = fields
		.iter()
		.map(|field| field.type_name.len())
		.max()
		.unwrap_or_default();
	let payload = if fields.is_empty() {
		"nil".to_owned()
	} else {
		let mut struct_text = "struct {\n".to_owned();
		for field in &fields {
			let omit = if field.optional { ",omitempty" } else { "" };
			struct_text.push_str(&format!(
				"\t{:width$} {:type_width$} `json:\"{}{omit}\"`\n",
				field.go_name, field.type_name, field.tag,
			));
		}
		struct_text.push_str("\t}{");
		let initializers: Vec<String> = fields
			.iter()
			.map(|field| {
				format!(
					"{}: {}",
					field.go_name,
					crate::naming::go_identifier(&field.tag)
				)
			})
			.collect();
		format!("{}{}}}", struct_text, initializers.join(", "))
	};
	let output = types.render(&function.output_schema, &format!("{hint}Result"), spec);
	let returns = if output == "struct{}" {
		"error".to_owned()
	} else {
		format!("({output}, error)")
	};
	Method {
		name: type_name(&function.name),
		parameters,
		encoding: match function.input {
			HttpInput::Query => "query",
			HttpInput::Form => "form",
			HttpInput::Json | HttpInput::Other { .. } => "json",
		},
		payload,
		returns,
	}
}

fn client_file(context: &Context<'_>, package: &str, types: &mut GoTypes) -> String {
	let mut methods = String::new();
	for function in context.functions() {
		let method = method(function, types, context.spec);
		let mut docs: Vec<String> = function
			.docs
			.lines()
			.map(str::trim)
			.map(str::to_owned)
			.collect();
		if docs.is_empty() {
			docs.push(method.name.clone());
		} else {
			docs[0] = format!("{} {}", method.name, docs[0].to_lowercase());
		}
		let documented: Vec<&crate::spec::ApiParam> = function
			.params
			.iter()
			.filter(|param| !param.docs.is_empty())
			.collect();
		if !documented.is_empty() {
			docs.push(String::new());
			for param in documented {
				docs.push(format!(
					"{} is {}",
					crate::naming::go_identifier(&param.name),
					param.docs.trim()
				));
			}
		}
		if function.deprecated {
			docs.push(String::new());
			docs.push("Deprecated: this function is going away.".to_owned());
		}
		for line in &docs {
			methods.push_str(&format!("// {line}\n"));
		}
		let decode = if method.returns == "error" {
			format!(
				"\treturn c.call(ctx, {}, {}, \"{}\", {})\n",
				go_string(&function.method),
				go_string(&function.path),
				method.encoding,
				method.payload
			)
		} else {
			format!(
				"\tvar result {result}\n\tif err := c.callInto(ctx, {method_}, {path}, \
				 \"{encoding}\", {payload}, &result); err != nil {{\n\t\treturn {zero}, \
				 err\n\t}}\n\treturn result, nil\n",
				result = method
					.returns
					.trim_matches(|c| c == '(' || c == ')')
					.trim_end_matches(", error"),
				method_ = go_string(&function.method),
				path = go_string(&function.path),
				encoding = method.encoding,
				payload = method.payload,
				zero = go_zero(&method.returns),
			)
		};
		methods.push_str(&format!(
			"func (c *Client) {}({}) {} {{\n{}}}\n\n",
			method.name,
			method.parameters.join(", "),
			method.returns,
			decode
		));
	}
	let mut out = context.banner("//");
	out.push_str(&format!(
		"// Code generated by leptoskit; DO NOT EDIT.\n\npackage {package}\n\nimport \
		 \"bytes\"\nimport \"context\"\nimport \"encoding/json\"\nimport \"fmt\"\nimport \
		 \"io\"\nimport \"net/http\"\nimport \"net/url\"\nimport \"strings\"\n\n// \
		 DefaultServerURL is the server origin baked in at generation time.\nconst \
		 DefaultServerURL = {server_url}\n\n{runtime}\nclass_placeholder",
		package = package,
		runtime = RUNTIME,
		server_url = go_string(
			context
				.server_url
				.as_deref()
				.unwrap_or("https://example.com")
		),
	));
	out = out.replace("class_placeholder", &methods);
	out
}

/// The zero value expression for a result type.
fn go_zero(returns: &str) -> String {
	let inner = returns
		.trim_start_matches('(')
		.trim_end_matches(')')
		.trim_end_matches(", error");
	if inner == "error" {
		return "nil".to_owned();
	}
	if inner.starts_with('*')
		|| inner.starts_with("[]")
		|| inner.starts_with("map[")
		|| inner == "any"
	{
		return "nil".to_owned();
	}
	format!("{inner}{{}}")
}

fn go_mod(context: &Context<'_>) -> String {
	format!(
		"{}module {}\n\ngo 1.22\n",
		context.banner("//"),
		context.options.package_name
	)
}

fn readme(context: &Context<'_>, package: &str) -> String {
	let example = context
		.functions()
		.next()
		.map_or_else(String::new, |function| {
			let args: Vec<&str> = function.params.iter().map(|_| "/* ... */").collect();
			format!(
				"result, err := client.{}(ctx, {})\n",
				type_name(&function.name),
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

A typed Go client for the **{title} {version}** RPC, generated by `leptoskit fns generate --target \
		 go`.

It depends only on the standard library, and the calls go to the same routes the app itself uses. \
		 Rename the module path in `go.mod` to where you publish it.

## Usage

```go
client := {package}.NewClient({server_url})

{example}if err != nil {{
	if fnErr := new({package}.FnError); errors.As(err, &fnErr) && fnErr.Retryable() {{
		// a network or server failure: safe to try again
	}}
	return err
}}
```

`FnError.Kind` is one of `KindNetwork`, `KindInvalid`, `KindServer`, `KindDecode`, or `KindOther`, \
		 mirroring the server's classification; `Retryable` is true for network and server \
		 failures. Shapes Go cannot model yet (tagged unions) appear as `any` holding the decoded \
		 JSON.{skipped}",
		name = context.options.package_name,
		title = context.spec.title,
		version = context.spec.version,
		server_url = go_string(
			context
				.server_url
				.as_deref()
				.unwrap_or("https://example.com")
		),
	)
}

/// The transport, error type, and `serde_qs` encoder the package shares.
const RUNTIME: &str = r#"// FnErrorKind classifies a failed RPC call, mirroring leptoskit_fns::FnError.
type FnErrorKind string

const (
	KindNetwork FnErrorKind = "network"
	KindInvalid FnErrorKind = "invalid"
	KindServer  FnErrorKind = "server"
	KindDecode  FnErrorKind = "decode"
	KindOther   FnErrorKind = "other"
)

// FnError is a failed RPC call.
type FnError struct {
	// Kind is the classification.
	Kind FnErrorKind
	// Message is the human-readable message.
	Message string
	// Status is the HTTP status code, when a response arrived.
	Status int
	// Raw is the response body, when a response arrived.
	Raw string
}

// Error implements error.
func (e *FnError) Error() string {
	return fmt.Sprintf("%s error: %s", e.Kind, e.Message)
}

// Retryable reports whether retrying the call could plausibly succeed.
func (e *FnError) Retryable() bool {
	return e.Kind == KindNetwork || e.Kind == KindServer
}

func fnErrorFromResponse(status int, body string) *FnError {
	variant, message, found := strings.Cut(body, "|")
	if !found {
		text := body
		if text == "" {
			text = fmt.Sprintf("HTTP %d", status)
		}
		return &FnError{Kind: KindOther, Message: text, Status: status, Raw: body}
	}
	kind := KindOther
	switch variant {
	case "Request":
		kind = KindNetwork
	case "Args", "MissingArg":
		kind = KindInvalid
	case "ServerError", "MiddlewareError", "WrappedServerFn":
		kind = KindServer
	case "Deserialization", "Serialization":
		kind = KindDecode
	}
	return &FnError{Kind: kind, Message: message, Status: status, Raw: body}
}

// Client is a typed client for the RPC.
type Client struct {
	// ServerURL is the origin the RPC is served from, without a trailing slash.
	ServerURL string
	// HTTPClient sends the requests; http.DefaultClient when nil.
	HTTPClient *http.Client
}

// NewClient returns a client for serverURL, an origin such as
// https://api.example.com. An empty string means DefaultServerURL.
func NewClient(serverURL string) *Client {
	if serverURL == "" {
		serverURL = DefaultServerURL
	}
	return &Client{ServerURL: strings.TrimRight(serverURL, "/")}
}

func (c *Client) httpClient() *http.Client {
	if c.HTTPClient != nil {
		return c.HTTPClient
	}
	return http.DefaultClient
}

// call runs one RPC and returns the raw JSON result, for functions that
// return nothing.
func (c *Client) call(ctx context.Context, method string, path string, encoding string, payload any) error {
	_, err := c.exchange(ctx, method, path, encoding, payload)
	return err
}

// callInto runs one RPC and decodes the JSON result into result.
func (c *Client) callInto(ctx context.Context, method string, path string, encoding string, payload any, result any) error {
	body, err := c.exchange(ctx, method, path, encoding, payload)
	if err != nil {
		return err
	}
	if len(body) == 0 || string(body) == "null" {
		return nil
	}
	if err := json.Unmarshal(body, result); err != nil {
		return &FnError{Kind: KindDecode, Message: fmt.Sprintf("the response was not JSON: %v", err), Raw: string(body)}
	}
	return nil
}

func (c *Client) exchange(ctx context.Context, method string, path string, encoding string, payload any) ([]byte, error) {
	target := c.ServerURL + path
	var body []byte
	request, err := http.NewRequestWithContext(ctx, method, target, nil)
	if err != nil {
		return nil, &FnError{Kind: KindOther, Message: err.Error()}
	}
	request.Header.Set("Accept", "application/json")
	switch encoding {
	case "query":
		// RawQuery carries no leading `?` of its own.
		if encoded := toQueryValues(payload).Encode(); encoded != "" {
			if request.URL.RawQuery != "" {
				request.URL.RawQuery += "&"
			}
			request.URL.RawQuery += encoded
		}
	case "form":
		form := toQueryValues(payload).Encode()
		body = []byte(form)
		request.Header.Set("Content-Type", "application/x-www-form-urlencoded")
		if form != "" {
			request.Body = io.NopCloser(strings.NewReader(form))
			request.ContentLength = int64(len(form))
		}
	default:
		// Absent optional arguments are omitted, as serde_qs omits them: the
		// server fills defaulted arguments in, and Option arguments read a
		// missing key as None.
		if payload != nil {
			encoded, err := json.Marshal(payload)
			if err != nil {
				return nil, &FnError{Kind: KindInvalid, Message: err.Error()}
			}
			var present map[string]any
			if err := json.Unmarshal(encoded, &present); err == nil {
				for key, item := range present {
					if item == nil {
						delete(present, key)
					}
				}
				if encoded, err = json.Marshal(present); err != nil {
					return nil, &FnError{Kind: KindInvalid, Message: err.Error()}
				}
			}
			body = encoded
			request.Body = io.NopCloser(bytes.NewReader(encoded))
			request.ContentLength = int64(len(encoded))
			request.Header.Set("Content-Type", "application/json")
		}
	}
	response, err := c.httpClient().Do(request)
	if err != nil {
		return nil, &FnError{Kind: KindNetwork, Message: fmt.Sprintf("could not reach %s: %v", target, err)}
	}
	defer response.Body.Close()
	text, err := io.ReadAll(response.Body)
	if err != nil {
		return nil, &FnError{Kind: KindNetwork, Message: err.Error()}
	}
	if response.StatusCode >= 400 {
		return nil, fnErrorFromResponse(response.StatusCode, string(text))
	}
	if encoding == "form" && body == nil {
		_ = body
	}
	return text, nil
}

// toQueryValues encodes a value the way serde_qs does: nested keys in
// brackets, arrays by index, nil omitted.
func toQueryValues(value any) url.Values {
	values := url.Values{}
	var appendValue func(key string, item any)
	appendValue = func(key string, item any) {
		switch typed := item.(type) {
		case nil:
		case bool:
			values.Set(key, fmt.Sprintf("%t", typed))
		case string:
			values.Set(key, typed)
		case map[string]any:
			for field, nested := range typed {
				appendValue(fmt.Sprintf("%s[%s]", key, field), nested)
			}
		case []any:
			for index, nested := range typed {
				appendValue(fmt.Sprintf("%s[%d]", key, index), nested)
			}
		default:
			values.Set(key, fmt.Sprintf("%v", typed))
		}
	}
	if encoded, err := json.Marshal(value); err == nil && string(encoded) != "null" {
		var tree map[string]any
		if err := json.Unmarshal(encoded, &tree); err == nil {
			for key, item := range tree {
				appendValue(key, item)
			}
		}
	}
	return values
}
"#;
