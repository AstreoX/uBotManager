//! The real package index of the uBot workspace: the embedded copy and (when present) uBotCore's
//! `Index/index.json` parse and validate without errors, and match `Index/index.schema.json`.
//!
//! uBotCore is looked up in `UBOTCORE_DIR`, else at `../../../../Plugins/uBot/uBotCore` relative to
//! this crate (the uBot workspace layout); the checks that need it are skipped without it.

use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::{json, Value};
use ubot_core::index;
use ubot_core::sources::EMBEDDED_INDEX;

fn ubotcore_dir() -> PathBuf {
    match std::env::var_os("UBOTCORE_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../Plugins/uBot/uBotCore"),
    }
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|text| text.trim_start_matches('\u{feff}').to_string())
}

#[test]
fn embedded_index_validates() {
    let validation = index::validate_text(EMBEDDED_INDEX);
    assert!(validation.errors.is_empty(), "{:#?}", validation.errors);
    let parsed = index::parse(EMBEDDED_INDEX).unwrap();
    for name in ["UBotCore", "UBotSensor", "UBotROS"] {
        let package = parsed.find(name).unwrap();
        assert!(!package.is_planned(), "{name} is released");
    }
}

#[test]
fn ubotcore_index_validates_and_matches_the_embedded_copy() {
    let Some(text) = read(&ubotcore_dir().join("Index").join("index.json")) else {
        eprintln!(
            "skipping: no uBotCore checkout at {}",
            ubotcore_dir().display()
        );
        return;
    };
    let validation = index::validate_text(&text);
    assert!(validation.errors.is_empty(), "{:#?}", validation.errors);
    assert!(validation.warnings.is_empty(), "{:#?}", validation.warnings);
    assert_eq!(
        text.replace("\r\n", "\n"),
        EMBEDDED_INDEX
            .trim_start_matches('\u{feff}')
            .replace("\r\n", "\n"),
        "assets/index.json is out of date; run node scripts/sync-index.mjs"
    );
}

#[test]
fn ubotcore_index_matches_its_schema() {
    let dir = ubotcore_dir().join("Index");
    let (Some(schema_text), Some(index_text)) = (
        read(&dir.join("index.schema.json")),
        read(&dir.join("index.json")),
    ) else {
        eprintln!(
            "skipping: no uBotCore checkout at {}",
            ubotcore_dir().display()
        );
        return;
    };
    let schema: Value = serde_json::from_str(&schema_text).unwrap();
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    let validator = Validator { root: &schema };

    let real: Value = serde_json::from_str(&index_text).unwrap();
    let errors = validator.validate(&real);
    assert!(errors.is_empty(), "{errors:#?}");
    let embedded: Value =
        serde_json::from_str(EMBEDDED_INDEX.trim_start_matches('\u{feff}')).unwrap();
    assert!(validator.validate(&embedded).is_empty());

    // The schema rejects what the reader rejects.
    let mut broken = real.clone();
    broken["formatVersion"] = json!(1);
    broken["packages"][0]["versions"][0]
        .as_object_mut()
        .unwrap()
        .remove("ref");
    broken["packages"][1]["layer"] = json!("Middleware");
    broken["packages"][1]["extra"] = json!(true);
    broken["packages"][2]["repository"] = json!("ext::sh -c x");
    broken["packages"][2]["versions"][0]["requires"][0]["version"] = json!(">=1.0,<2.0");
    broken["packages"][3]
        .as_object_mut()
        .unwrap()
        .remove("repository");
    broken["packages"][3]["versions"][0]["planned"] = json!(false);
    broken["recipes"][0]["packages"] = json!([]);
    broken["replacements"][0]["redirects"] = json!("../outside.ini");
    let errors = validator.validate(&broken);
    for fragment in [
        "/formatVersion",
        "/packages/0/versions/0: missing \"ref\"",
        "/packages/1/layer",
        "/packages/1: unknown property \"extra\"",
        "/packages/2/repository",
        "/packages/2/versions/0/requires/0/version",
        "/packages/3: missing \"repository\"",
        "/recipes/0/packages",
        "/replacements/0/redirects",
    ] {
        assert!(
            errors.iter().any(|error| error.starts_with(fragment)),
            "expected an error at {fragment}, got {errors:#?}"
        );
    }

    // A quote in a ref is rejected by the reader and by the schema alike.
    let mut quoted = real.clone();
    quoted["packages"][0]["versions"][0]["ref"] = json!("v1\"");
    let errors = validator.validate(&quoted);
    assert!(
        errors
            .iter()
            .any(|error| error.starts_with("/packages/0/versions/0/ref")),
        "{errors:#?}"
    );

    // Prebuilt binaries: the shapes `ubot index validate` accepts, and what it refuses.
    let sha = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let mut binaries = real.clone();
    binaries["packages"][0]["versions"][0]["binaries"] = json!({
        "5.5": { "Win64": { "url": "https://github.com/AstreoX/uBotCore/releases/download/v0.1.0/UBotCore-0.1.0-UE5.5-Win64.zip", "sha256": sha, "size": 1234 } },
        "5.6": { "Win64": { "url": "file:///D:/mirror/UBotCore.zip", "sha256": sha },
                 "Linux": { "url": "/srv/mirror/UBotCore.zip", "sha256": sha } },
        "5.7": { "Win64": { "url": "D:\\mirror\\UBotCore.zip", "sha256": sha } },
        "5.8": { "Win64": { "url": "\\\\server\\share\\UBotCore.zip", "sha256": sha } }
    });
    let errors = validator.validate(&binaries);
    assert!(errors.is_empty(), "{errors:#?}");
    let text = serde_json::to_string(&binaries).unwrap();
    let validation = index::validate_text(&text);
    assert!(validation.errors.is_empty(), "{:#?}", validation.errors);
    binaries["packages"][0]["versions"][0]["binaries"] = Value::Null;
    assert!(validator.validate(&binaries).is_empty(), "null means none");
    for (bad, fragment) in [
        (
            json!({ "5": { "Win64": { "url": "https://h/x.zip", "sha256": sha } } }),
            "/binaries/5 (name)",
        ),
        (json!({ "5.5.1": {} }), "/binaries/5.5.1 (name)"),
        (
            json!({ "5.5": { "Win64": { "url": "http://h/x.zip", "sha256": sha } } }),
            "/binaries/5.5/Win64/url",
        ),
        (
            json!({ "5.5": { "Win64": { "url": "https://h/a b.zip", "sha256": sha } } }),
            "/binaries/5.5/Win64/url",
        ),
        (
            json!({ "5.5": { "Win64": { "url": "relative/x.zip", "sha256": sha } } }),
            "/binaries/5.5/Win64/url",
        ),
        (
            json!({ "5.5": { "Win64": { "url": "https://h/x.zip", "sha256": sha.to_uppercase() } } }),
            "/binaries/5.5/Win64/sha256",
        ),
        (
            json!({ "5.5": { "Win64": { "url": "https://h/x.zip" } } }),
            "/binaries/5.5/Win64: missing \"sha256\"",
        ),
        (
            json!({ "5.5": { "Win64": { "url": "https://h/x.zip", "sha256": sha, "size": 0 } } }),
            "/binaries/5.5/Win64/size",
        ),
        (
            json!({ "5.5": { "Win64": { "url": "https://h/x.zip", "sha256": sha, "size": 3_000_000_000u64 } } }),
            "/binaries/5.5/Win64/size",
        ),
        (
            json!({ "5.5": { "Win64": { "url": "https://h/x.zip", "sha256": sha, "mirror": "x" } } }),
            "/binaries/5.5/Win64: unknown property \"mirror\"",
        ),
        (json!({ "5.5": "x" }), "/binaries/5.5"),
        (json!([]), "/binaries"),
    ] {
        let mut broken = real.clone();
        broken["packages"][0]["versions"][0]["binaries"] = bad.clone();
        let errors = validator.validate(&broken);
        assert!(
            errors
                .iter()
                .any(|error| error.starts_with(&format!("/packages/0/versions/0{fragment}"))),
            "{bad}: expected an error at {fragment}, got {errors:#?}"
        );
        // `ubot index validate` refuses it too (an unknown key is only a warning there).
        let validation = index::validate_text(&serde_json::to_string(&broken).unwrap());
        if fragment.contains("unknown property") {
            assert!(!validation.warnings.is_empty(), "{bad}");
        } else {
            assert!(!validation.errors.is_empty(), "{bad}");
        }
    }

    // A minimal index and every constraint form the reader accepts.
    let mut minimal = json!({ "formatVersion": 2, "packages": [ { "name": "A", "versions": [ { "version": "1.0.0", "planned": true } ] } ] });
    assert!(
        validator.validate(&minimal).is_empty(),
        "a planned package needs no repository"
    );
    for constraint in [
        "",
        "*",
        "^0.1.0",
        ">= 1.0 < 2.0",
        "~5.5",
        "=1.2.3-beta.1",
        "v1",
        "* >=1.0",
    ] {
        minimal["packages"][0]["versions"][0]["requires"] =
            json!([ { "name": "A", "version": constraint } ]);
        assert!(
            validator.validate(&minimal).is_empty(),
            "'{constraint}' is accepted"
        );
    }
    for constraint in [">=", "1.x", "latest", ">=1.0,<2.0", "!=1.0"] {
        minimal["packages"][0]["versions"][0]["requires"] =
            json!([ { "name": "A", "version": constraint } ]);
        assert!(
            !validator.validate(&minimal).is_empty(),
            "'{constraint}' is rejected"
        );
    }
}

/// The subset of JSON Schema 2020-12 that `index.schema.json` uses.
struct Validator<'a> {
    root: &'a Value,
}

impl Validator<'_> {
    fn validate(&self, instance: &Value) -> Vec<String> {
        let mut errors = Vec::new();
        self.check(self.root, instance, "", &mut errors);
        errors
    }

    fn is_valid(&self, schema: &Value, instance: &Value) -> bool {
        let mut errors = Vec::new();
        self.check(schema, instance, "", &mut errors);
        errors.is_empty()
    }

    fn resolve<'s>(&'s self, reference: &str) -> &'s Value {
        let pointer = reference
            .strip_prefix('#')
            .expect("only local references are used");
        self.root
            .pointer(pointer)
            .unwrap_or_else(|| panic!("unresolved $ref {reference}"))
    }

    fn check(&self, schema: &Value, instance: &Value, at: &str, errors: &mut Vec<String>) {
        let Some(schema) = schema.as_object() else {
            if schema == &Value::Bool(false) {
                errors.push(format!("{at}: not allowed"));
            }
            return;
        };
        let mut own: Vec<String> = Vec::new();
        let mut fail = |message: String| own.push(format!("{at}: {message}"));
        let known = [
            "$schema",
            "$id",
            "$defs",
            "title",
            "description",
            "$ref",
            "type",
            "const",
            "enum",
            "required",
            "properties",
            "additionalProperties",
            "items",
            "minItems",
            "uniqueItems",
            "minLength",
            "pattern",
            "not",
            "if",
            "then",
            "else",
            "contains",
            "propertyNames",
            "minimum",
            "maximum",
        ];
        for keyword in schema.keys() {
            assert!(
                known.contains(&keyword.as_str()),
                "the test validator does not implement '{keyword}'"
            );
        }

        let mut errors_here = Vec::new();
        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            self.check(self.resolve(reference), instance, at, &mut errors_here);
        }
        if let Some(types) = schema.get("type") {
            let types: Vec<&str> = match types {
                Value::String(name) => vec![name.as_str()],
                Value::Array(names) => names.iter().filter_map(Value::as_str).collect(),
                _ => vec![],
            };
            let matches = types.iter().any(|name| match *name {
                "object" => instance.is_object(),
                "array" => instance.is_array(),
                "string" => instance.is_string(),
                "boolean" => instance.is_boolean(),
                "null" => instance.is_null(),
                "integer" => instance.is_i64() || instance.is_u64(),
                "number" => instance.is_number(),
                other => panic!("unknown type {other}"),
            });
            if !matches {
                errors.push(format!("{at}: expected {types:?}"));
                return;
            }
        }
        if let Some(constant) = schema.get("const") {
            if constant != instance {
                fail(format!("expected {constant}"));
            }
        }
        if let Some(options) = schema.get("enum").and_then(Value::as_array) {
            if !options.contains(instance) {
                fail(format!("{instance} is not one of {options:?}"));
            }
        }
        if let Some(text) = instance.as_str() {
            if let Some(min) = schema.get("minLength").and_then(Value::as_u64) {
                if (text.chars().count() as u64) < min {
                    fail(format!("shorter than {min}"));
                }
            }
            if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
                if !Regex::new(pattern).unwrap().is_match(text) {
                    fail(format!("'{text}' does not match {pattern}"));
                }
            }
        }
        if let Some(number) = instance.as_f64() {
            if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64) {
                if number < minimum {
                    fail(format!("{number} is less than {minimum}"));
                }
            }
            if let Some(maximum) = schema.get("maximum").and_then(Value::as_f64) {
                if number > maximum {
                    fail(format!("{number} is more than {maximum}"));
                }
            }
        }
        if let Some(items) = instance.as_array() {
            if let Some(min) = schema.get("minItems").and_then(Value::as_u64) {
                if (items.len() as u64) < min {
                    fail(format!("fewer than {min} items"));
                }
            }
            if schema.get("uniqueItems") == Some(&Value::Bool(true)) {
                for (index, item) in items.iter().enumerate() {
                    if items[..index].contains(item) {
                        fail(format!("duplicate item {item}"));
                    }
                }
            }
            if let Some(item_schema) = schema.get("items") {
                for (index, item) in items.iter().enumerate() {
                    self.check(
                        item_schema,
                        item,
                        &format!("{at}/{index}"),
                        &mut errors_here,
                    );
                }
            }
            if let Some(contains) = schema.get("contains") {
                if !items.iter().any(|item| self.is_valid(contains, item)) {
                    fail("no item matches \"contains\"".into());
                }
            }
        }
        if let Some(object) = instance.as_object() {
            if let Some(required) = schema.get("required").and_then(Value::as_array) {
                for name in required.iter().filter_map(Value::as_str) {
                    if !object.contains_key(name) {
                        fail(format!("missing \"{name}\""));
                    }
                }
            }
            if let Some(names) = schema.get("propertyNames") {
                for name in object.keys() {
                    self.check(
                        names,
                        &Value::String(name.clone()),
                        &format!("{at}/{name} (name)"),
                        &mut errors_here,
                    );
                }
            }
            let properties = schema.get("properties").and_then(Value::as_object);
            for (name, value) in object {
                let child = format!("{at}/{name}");
                match properties.and_then(|properties| properties.get(name)) {
                    Some(property) => self.check(property, value, &child, &mut errors_here),
                    None => match schema.get("additionalProperties") {
                        Some(Value::Bool(false)) => fail(format!("unknown property \"{name}\"")),
                        Some(additional @ Value::Object(_)) => {
                            self.check(additional, value, &child, &mut errors_here)
                        }
                        _ => {}
                    },
                }
            }
        }
        if let Some(not) = schema.get("not") {
            if self.is_valid(not, instance) {
                fail("matches a schema it must not match".into());
            }
        }
        if let Some(condition) = schema.get("if") {
            let branch = if self.is_valid(condition, instance) {
                schema.get("then")
            } else {
                schema.get("else")
            };
            if let Some(branch) = branch {
                self.check(branch, instance, at, &mut errors_here);
            }
        }
        errors.extend(own);
        errors.extend(errors_here);
    }
}
