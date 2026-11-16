//! Writes `packages/viewer/src/protocol.ts` from the JSON Schemas of the viewer messages, and
//! checks that the committed file is that output. `ORBE_BLESS=1` rewrites it, which
//! `scripts/bless.sh` does.
//!
//! A `$ref` into another file, such as `./view#/$defs/View`, is a type that file declares by
//! hand, imported and re-exported.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::live::WorldName;
use crate::target::Ref;
use crate::viewer::{ClosedMessage, FrameMessage, ReplyMessage, SENT, SceneMessage};

/// Everything a viewer gets from its session's source.
#[derive(JsonSchema)]
#[serde(untagged)]
#[expect(dead_code, reason = "only its schema is read")]
enum ToViewer<'a> {
    Scene(SceneMessage<'a>),
    Frame(FrameMessage<'a>),
    Closed(ClosedMessage<'a>),
}

/// Everything a session's WebSocket sends.
#[derive(JsonSchema)]
#[serde(untagged)]
#[expect(dead_code, reason = "only its schema is read")]
enum FromSession<'a> {
    ToViewer(ToViewer<'a>),
    Reply(ReplyMessage<'a>),
}

/// A command the viewer sends. The session reads it through the CLI's own table, as
/// `orbe press @btn1` would be.
#[derive(Serialize, JsonSchema)]
#[serde(tag = "cmd", rename_all = "lowercase")]
enum Command {
    Press { r#ref: Ref },
    Release { r#ref: Ref },
    Play,
    Pause,
    Set { quantity: WorldName, value: f64 },
}

/// A command as the viewer sends it on a session's WebSocket. `id` is the viewer's own and comes
/// back on its reply.
#[derive(Serialize, JsonSchema)]
#[schemars(extend("allOf" = [{ "$ref": "#/$defs/Command" }]))]
struct CommandMessage {
    id: u64,
    #[serde(flatten)]
    #[schemars(skip)]
    command: Command,
}

const HEADER: &str = "// Generated from the viewer messages in packages/session. Edit those, then run scripts/bless.sh.\n";

/// The column a doc comment wraps at.
const WIDTH: usize = 100;

fn typescript() -> String {
    let mut generator = SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator();
    generator.subschema_for::<FromSession>();
    generator.subschema_for::<Command>();
    generator.subschema_for::<CommandMessage>();
    let defs = generator.take_definitions(true);
    let mut ts = Ts::default();
    let body: Vec<String> = defs
        .iter()
        .map(|(name, schema)| ts.declaration(name, schema))
        .collect();
    let mut out = HEADER.to_string();
    for (module, names) in &ts.imports {
        let names = names.iter().cloned().collect::<Vec<_>>().join(", ");
        let _ = writeln!(out, "import type {{ {names} }} from \"{module}\";");
        let _ = writeln!(out, "export type * from \"{module}\";");
    }
    for declaration in body {
        out.push('\n');
        out.push_str(&declaration);
    }
    out
}

/// The TypeScript of each schema, and the types it imports by module.
#[derive(Default)]
struct Ts {
    imports: BTreeMap<String, BTreeSet<String>>,
}

impl Ts {
    fn declaration(&mut self, name: &str, schema: &Value) -> String {
        let mut out = doc(schema, "");
        let object = schema.as_object().expect("a definition is an object");
        let extends: Vec<String> = object
            .get("allOf")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|part| self.reference(part["$ref"].as_str().expect("allOf holds $refs")))
            .collect();
        let union = object.get("anyOf").or_else(|| object.get("oneOf"));
        let fields = object.contains_key("properties");
        if fields && union.is_none() && extends.is_empty() {
            let _ = writeln!(out, "export interface {name} {}", self.fields(object, ""));
        } else if let Some(members) = union.filter(|m| {
            let one_line = format!("export type {name} = {};", self.union(m));
            !fields && extends.is_empty() && (documented(m) || one_line.len() > WIDTH)
        }) {
            let _ = writeln!(out, "export type {name} =");
            let members = members.as_array().expect("a union is an array");
            for (i, member) in members.iter().enumerate() {
                out.push_str(&doc(member, "  "));
                let end = if i + 1 == members.len() { ";" } else { "" };
                let _ = writeln!(out, "  | {}{end}", self.ty(member));
            }
        } else {
            let mut parts = extends;
            if fields {
                parts.push(self.fields(object, ""));
            }
            match union {
                Some(members) if parts.is_empty() => parts.push(self.union(members)),
                Some(members) => parts.push(format!("({})", self.union(members))),
                None if parts.is_empty() => parts.push(self.ty(schema)),
                None => {}
            }
            let _ = writeln!(out, "export type {name} = {};", parts.join(" & "));
        }
        out
    }

    /// An object type's fields, one per line at `indent`, each with its doc.
    fn fields(&mut self, object: &Map<String, Value>, indent: &str) -> String {
        let mut out = String::from("{\n");
        for (field, line) in self.field_lines(object) {
            out.push_str(&doc(field, &format!("{indent}  ")));
            let _ = writeln!(out, "{indent}  {line};");
        }
        out.push_str(indent);
        out.push('}');
        out
    }

    /// An object type on one line, without its docs.
    fn inline(&mut self, object: &Map<String, Value>) -> String {
        let lines: Vec<String> = self
            .field_lines(object)
            .into_iter()
            .map(|(_, line)| line)
            .collect();
        format!("{{ {} }}", lines.join("; "))
    }

    /// Each field's schema and its declaration, such as `readonly ratio?: number`. A field the
    /// object may leave out is optional, and is never null.
    fn field_lines<'s>(&mut self, object: &'s Map<String, Value>) -> Vec<(&'s Value, String)> {
        let required: BTreeSet<&str> = object
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        object["properties"]
            .as_object()
            .expect("properties is an object")
            .iter()
            .map(|(key, field)| {
                let ty = self.ty(field);
                let line = if required.contains(key.as_str()) {
                    format!("readonly {key}: {ty}")
                } else {
                    let ty = ty.strip_suffix(" | null").unwrap_or(&ty);
                    format!("readonly {key}?: {ty}")
                };
                (field, line)
            })
            .collect()
    }

    fn union(&mut self, members: &Value) -> String {
        members
            .as_array()
            .expect("a union is an array")
            .iter()
            .map(|member| self.ty(member))
            .collect::<Vec<_>>()
            .join(" | ")
    }

    fn reference(&mut self, target: &str) -> String {
        let (module, pointer) = target.split_once('#').expect("a $ref has a #");
        let name = pointer
            .strip_prefix("/$defs/")
            .expect("a $ref points into $defs");
        if !module.is_empty() {
            self.imports
                .entry(module.to_string())
                .or_default()
                .insert(name.to_string());
        }
        name.to_string()
    }

    fn ty(&mut self, schema: &Value) -> String {
        if schema == &Value::Bool(true) {
            return "unknown".into();
        }
        let object = schema.as_object().expect("a schema is an object or true");
        if let Some(target) = object.get("$ref").and_then(Value::as_str) {
            return self.reference(target);
        }
        if let Some(value) = object.get("const") {
            return value.to_string();
        }
        if let Some(values) = object.get("enum").and_then(Value::as_array) {
            return values
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join(" | ");
        }
        if let Some(members) = object.get("anyOf").or_else(|| object.get("oneOf")) {
            return self.union(members);
        }
        let types: Vec<&str> = match &object["type"] {
            Value::String(one) => vec![one.as_str()],
            Value::Array(many) => many.iter().filter_map(Value::as_str).collect(),
            other => panic!("no TypeScript for the type {other} in {schema}"),
        };
        types
            .into_iter()
            .map(|ty| match ty {
                "string" => match object.get("pattern").and_then(Value::as_str) {
                    Some(pattern) => {
                        let prefix = pattern.strip_prefix('^').expect("a pattern is a prefix");
                        format!("`{prefix}${{string}}`")
                    }
                    None => "string".into(),
                },
                "integer" | "number" => "number".into(),
                "boolean" => "boolean".into(),
                "null" => "null".into(),
                "array" => {
                    let item = self.ty(&object["items"]);
                    if item.contains(' ') {
                        format!("readonly ({item})[]")
                    } else {
                        format!("readonly {item}[]")
                    }
                }
                "object" if object.contains_key("properties") => self.inline(object),
                "object" => {
                    let value = object
                        .get("additionalProperties")
                        .unwrap_or(&Value::Bool(true));
                    format!("Readonly<Record<string, {}>>", self.ty(value))
                }
                other => panic!("no TypeScript for the type {other} in {schema}"),
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

/// Whether any member of a union has a doc. A union with one, or one too long for a line, is
/// written one member per line.
fn documented(members: &Value) -> bool {
    members
        .as_array()
        .is_some_and(|members| members.iter().any(|m| m.get("description").is_some()))
}

/// A schema's `description` as a doc comment at `indent`, wrapped at `WIDTH`.
fn doc(schema: &Value, indent: &str) -> String {
    let Some(text) = schema.get("description").and_then(Value::as_str) else {
        return String::new();
    };
    let mut lines = vec![format!("{indent}/**")];
    for word in text.split_whitespace() {
        let last = lines.last_mut().expect("a first line");
        if last.len() + 1 + word.len() > WIDTH - 3 && last.trim() != "/**" && last.trim() != "*" {
            lines.push(format!("{indent} * {word}"));
        } else {
            last.push(' ');
            last.push_str(word);
        }
    }
    let last = lines.last_mut().expect("a first line");
    if last.len() + 3 > WIDTH {
        lines.push(format!("{indent} */"));
    } else {
        last.push_str(" */");
    }
    lines.join("\n") + "\n"
}

#[test]
fn protocol_ts_is_the_generated_one() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../viewer/src/protocol.ts");
    let generated = typescript();
    if std::env::var_os("ORBE_BLESS").is_some() {
        std::fs::write(&path, &generated).unwrap();
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "packages/viewer/src/protocol.ts is not the file the viewer messages generate; run scripts/bless.sh"
    );
}

/// Each command the viewer's type allows reads as the CLI reads its line, and the type has one
/// per command a viewer sends.
#[test]
fn every_command_the_type_allows_is_read_as_the_cli_reads_it() {
    let btn = Ref::parse("@btn1").unwrap();
    let commands = [
        (Command::Press { r#ref: btn.clone() }, "press @btn1"),
        (Command::Release { r#ref: btn }, "release @btn1"),
        (Command::Play, "play"),
        (Command::Pause, "pause"),
        (
            Command::Set {
                quantity: WorldName::Light,
                value: 50.0,
            },
            "set light 50",
        ),
    ];
    let names: Vec<&str> = commands
        .iter()
        .map(|(_, line)| line.split(' ').next().unwrap())
        .collect();
    assert_eq!(names, SENT);
    for (id, (command, line)) in (1..).zip(commands) {
        let text = serde_json::to_string(&CommandMessage { id, command }).unwrap();
        let (read, parsed) = crate::viewer::command(&text);
        assert_eq!(read, Some(id), "{text}");
        assert_eq!(
            parsed.map_err(|error| error.message),
            Ok(crate::Command::parse(line, Path::new("")).unwrap().unwrap()),
            "{text}"
        );
    }
}
