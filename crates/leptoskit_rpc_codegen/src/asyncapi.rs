//! The live side of the RPC as an `AsyncAPI` 3.0 document: the operations a
//! client subscribes to and sends over the live socket.
//!
//! The HTTP one-shots stay OpenAPI's business; this describes the same
//! operations as messages, so event-driven tooling can read them.

use serde_json::Map;
use serde_json::Value;
use serde_json::json;

use crate::ApiKind;
use crate::ApiParam;
use crate::ApiSpec;

impl ApiSpec {
	/// Renders the live operations as an `AsyncAPI` 3.0 document.
	///
	/// A query is a `receive` operation (subscribe); a mutation or action is
	/// a `send` one. Calls have no live form and are left out, as are
	/// internal functions.
	#[must_use]
	pub fn to_asyncapi(&self) -> Value {
		let mut channels = Map::new();
		let mut operations = Map::new();
		let mut messages = Map::new();

		for function in &self.functions {
			if function.internal || function.kind == ApiKind::Call {
				continue;
			}

			let id = &function.name;
			let args = format!("{id}.args");
			let result = format!("{id}.result");
			messages.insert(
				args.clone(),
				json!({
					"name": args,
					"title": format!("{}'s arguments", function.name),
					"payload": object_schema(&function.params),
				}),
			);
			messages.insert(
				result.clone(),
				json!({
					"name": result,
					"title": format!("{}'s result", function.name),
					"payload": function.output_schema,
				}),
			);

			channels.insert(
				id.clone(),
				json!({
					"address": id,
					"title": function.docs,
					"messages": {
						"args": { "$ref": format!("#/components/messages/{args}") },
						"result": { "$ref": format!("#/components/messages/{result}") },
					},
				}),
			);

			let (operation, action, message) = match function.kind {
				ApiKind::Query => ("subscribe", "receive", "result"),
				ApiKind::Mutation | ApiKind::Action => ("call", "send", "args"),
				ApiKind::Call => continue,
			};

			operations.insert(
				format!("{id}.{operation}"),
				json!({
					"action": action,
					"channel": { "$ref": format!("#/channels/{id}") },
					"messages": [{ "$ref": format!("#/channels/{id}/messages/{message}") }],
				}),
			);
		}

		let mut document = Map::new();
		document.insert("asyncapi".to_owned(), json!("3.0.0"));
		let mut info = Map::new();
		info.insert("title".to_owned(), json!(self.title));
		info.insert("version".to_owned(), json!(self.version));

		if let Some(description) = &self.description {
			info.insert("description".to_owned(), json!(description));
		}

		document.insert("info".to_owned(), Value::Object(info));

		if let Some(server) = self.servers.first() {
			document.insert(
				"servers".to_owned(),
				json!({
					"live": {
						"host": server,
						"protocol": if server.starts_with("https") { "wss" } else { "ws" },
						"pathname": "/live",
					}
				}),
			);
		}

		document.insert("channels".to_owned(), Value::Object(channels));
		document.insert("operations".to_owned(), Value::Object(operations));
		document.insert(
			"components".to_owned(),
			json!({
				"messages": messages,
				"schemas": self.schemas,
			}),
		);
		Value::Object(document)
	}
}

/// One operation's arguments as a JSON Schema object.
fn object_schema(params: &[ApiParam]) -> Value {
	let mut properties = Map::new();
	let mut required = Vec::new();

	for param in params {
		properties.insert(param.name.clone(), param.schema.clone());

		if param.is_required() {
			required.push(Value::String(param.name.clone()));
		}
	}

	let mut schema = Map::new();
	schema.insert("type".to_owned(), json!("object"));

	schema.insert("properties".to_owned(), Value::Object(properties));

	if !required.is_empty() {
		schema.insert("required".to_owned(), Value::Array(required));
	}

	Value::Object(schema)
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use crate::ApiFunction;
	use crate::ApiKind;
	use crate::ApiParam;
	use crate::ApiSpec;
	use crate::HttpInput;
	use crate::HttpOutput;

	fn spec() -> ApiSpec {
		let mut spec = ApiSpec::empty("Notes", "1.0.0");
		spec.functions = vec![
			ApiFunction {
				name: "notes".to_owned(),
				kind: ApiKind::Query,
				module_path: "notes".to_owned(),
				struct_name: "Notes".to_owned(),
				docs: "Every note.".to_owned(),
				path: "/api/notes".to_owned(),
				method: "POST".to_owned(),
				input: HttpInput::Json,
				output: HttpOutput::Json,
				error_content_type: "text/plain".to_owned(),
				params: vec![ApiParam {
					name: "limit".to_owned(),
					docs: String::new(),
					schema: json!({ "type": "integer" }),
					required: true,
				}],
				output_schema: json!({ "type": "array", "items": { "type": "string" } }),
				internal: false,
				deprecated: false,
				tags: Vec::new(),
				scopes: Vec::new(),
			},
			ApiFunction {
				name: "add_note".to_owned(),
				kind: ApiKind::Mutation,
				module_path: "notes".to_owned(),
				struct_name: "AddNote".to_owned(),
				docs: String::new(),
				path: "/api/add_note".to_owned(),
				method: "POST".to_owned(),
				input: HttpInput::Json,
				output: HttpOutput::Json,
				error_content_type: "text/plain".to_owned(),
				params: vec![ApiParam {
					name: "body".to_owned(),
					docs: String::new(),
					schema: json!({ "type": "string" }),
					required: true,
				}],
				output_schema: json!({ "type": "string" }),
				internal: false,
				deprecated: false,
				tags: Vec::new(),
				scopes: Vec::new(),
			},
		];
		spec
	}

	#[test]
	fn queries_receive_and_mutations_send() {
		let document = spec().to_asyncapi();
		assert_eq!(document["asyncapi"], "3.0.0");
		assert_eq!(document["channels"]["notes"]["address"], "notes");
		assert_eq!(
			document["operations"]["notes.subscribe"],
			json!({
				"action": "receive",
				"channel": { "$ref": "#/channels/notes" },
				"messages": [{ "$ref": "#/channels/notes/messages/result" }],
			})
		);
		assert_eq!(document["operations"]["add_note.call"]["action"], "send");
		assert_eq!(
			document["components"]["messages"]["add_note.args"]["payload"],
			json!({
				"type": "object",
				"properties": { "body": { "type": "string" } },
				"required": ["body"],
			})
		);
	}
}
