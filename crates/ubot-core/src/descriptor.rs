//! Plugin descriptors (`.uplugin`) and their `"UBot"` block (SPEC §3).
//!
//! Parsing is lenient like uBotCore's `FUBotPackageRegistry::ParseDescriptor`: malformed fields are
//! skipped and reported as issues, so a slightly broken package still shows up.

use serde_json::{Map, Value};

use crate::model::Layer;
use crate::version::{Constraint, SemVer};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dependency {
    pub name: String,
    pub version: String,
    pub optional: bool,
}

/// The `"UBot"` block of a descriptor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UBotBlock {
    pub layer: Layer,
    pub repository: Option<String>,
    pub requires: Vec<Dependency>,
    pub provides: Vec<String>,
    pub tags: Vec<String>,
    /// `EnginePlugins`, or the legacy key `ExternalRequires`.
    pub engine_plugins: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Descriptor {
    pub version_name: Option<String>,
    pub friendly_name: Option<String>,
    pub description: Option<String>,
    pub docs_url: Option<String>,
    pub enabled_by_default: Option<bool>,
    /// `Some` when the descriptor has a `"UBot"` object: the plugin is a uBot package.
    pub ubot: Option<UBotBlock>,
    /// Problems found while reading the descriptor (only collected for uBot packages).
    pub issues: Vec<String>,
}

impl Descriptor {
    pub fn is_ubot(&self) -> bool {
        self.ubot.is_some()
    }
}

/// Parses descriptor text. `Err` only when the text is not a JSON object.
pub fn parse(text: &str) -> Result<Descriptor, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("invalid JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or("the descriptor is not a JSON object")?;
    Ok(parse_object(object))
}

pub fn parse_object(json: &Map<String, Value>) -> Descriptor {
    let mut issues = Vec::new();
    let mut descriptor = Descriptor {
        version_name: read_string(json, "VersionName", &mut issues),
        friendly_name: read_string(json, "FriendlyName", &mut issues),
        description: read_string(json, "Description", &mut issues),
        docs_url: read_string(json, "DocsURL", &mut issues),
        enabled_by_default: match json.get("EnabledByDefault") {
            Some(Value::Bool(value)) => Some(*value),
            Some(_) => {
                issues.push("\"EnabledByDefault\" is not a boolean.".into());
                None
            }
            None => None,
        },
        ubot: None,
        issues: Vec::new(),
    };

    match json.get("UBot") {
        None => return descriptor,
        Some(Value::Object(block)) => {
            match descriptor.version_name.as_deref() {
                None | Some("") => issues.push("The descriptor has no VersionName.".into()),
                Some(version) if SemVer::parse(version).is_none() => issues.push(format!(
                    "VersionName '{version}' is not a semantic version."
                )),
                Some(_) => {}
            }
            descriptor.ubot = Some(parse_block(block, &mut issues));
        }
        Some(_) => {
            issues.push("The \"UBot\" field is not an object.".into());
            descriptor.ubot = Some(UBotBlock::default());
        }
    }
    descriptor.issues = issues;
    descriptor
}

fn parse_block(block: &Map<String, Value>, issues: &mut Vec<String>) -> UBotBlock {
    let mut result = UBotBlock::default();
    match block.get("Layer") {
        None => issues.push("The \"UBot\" block has no Layer.".into()),
        Some(Value::String(text)) => match Layer::parse(text) {
            Some(layer) => result.layer = layer,
            None => issues.push(format!(
                "Unknown layer '{}' (expected Foundation, Capability, Adapter or Content).",
                text.trim()
            )),
        },
        Some(_) => issues.push("\"Layer\" is not a string.".into()),
    }
    result.repository = read_string(block, "Repository", issues).filter(|text| !text.is_empty());
    result.tags = read_string_array(block, "Tags", issues);
    result.provides = read_string_array(block, "Provides", issues);
    result.engine_plugins = read_string_array(block, "EnginePlugins", issues);
    for legacy in read_string_array(block, "ExternalRequires", issues) {
        if !result
            .engine_plugins
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&legacy))
        {
            result.engine_plugins.push(legacy);
        }
    }
    result.requires = read_requires(block, issues);
    result
}

fn read_string(json: &Map<String, Value>, field: &str, issues: &mut Vec<String>) -> Option<String> {
    match json.get(field)? {
        Value::String(text) => Some(text.trim().to_string()),
        _ => {
            issues.push(format!("\"{field}\" is not a string."));
            None
        }
    }
}

fn read_string_array(
    json: &Map<String, Value>,
    field: &str,
    issues: &mut Vec<String>,
) -> Vec<String> {
    let mut values: Vec<String> = Vec::new();
    match json.get(field) {
        None => {}
        Some(Value::Array(items)) => {
            for (position, item) in items.iter().enumerate() {
                match item.as_str().map(str::trim) {
                    Some(text) if !text.is_empty() => {
                        if !values.iter().any(|existing| existing == text) {
                            values.push(text.to_string());
                        }
                    }
                    _ => issues.push(format!(
                        "\"{field}\"[{position}] is not a non-empty string."
                    )),
                }
            }
        }
        Some(_) => issues.push(format!("\"{field}\" is not an array.")),
    }
    values
}

fn read_requires(block: &Map<String, Value>, issues: &mut Vec<String>) -> Vec<Dependency> {
    let items = match block.get("Requires") {
        None => return Vec::new(),
        Some(Value::Array(items)) => items,
        Some(_) => {
            issues.push("\"Requires\" is not an array.".into());
            return Vec::new();
        }
    };

    let mut requires = Vec::new();
    for (position, item) in items.iter().enumerate() {
        let Some(entry) = item.as_object() else {
            issues.push(format!("\"Requires\"[{position}] is not an object."));
            continue;
        };
        let mut entry_issues = Vec::new();
        let name = read_string(entry, "Name", &mut entry_issues).unwrap_or_default();
        let version = read_string(entry, "Version", &mut entry_issues).unwrap_or_default();
        let optional = match entry.get("Optional") {
            None => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => {
                entry_issues.push("\"Optional\" is not a boolean.".to_string());
                false
            }
        };
        if name.is_empty() {
            entry_issues.push("it has no \"Name\".".into());
        }
        if !entry_issues.is_empty() {
            issues.push(format!(
                "\"Requires\"[{position}] was skipped: {}",
                entry_issues.join(" ")
            ));
            continue;
        }
        if let Err(error) = Constraint::parse(&version) {
            issues.push(format!(
                "Requirement '{name}' has an unparsable version constraint: {error}."
            ));
        }
        requires.push(Dependency {
            name,
            version,
            optional,
        });
    }
    requires
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_descriptor() {
        let descriptor = parse(
            r#"{
            "FileVersion": 3, "VersionName": "0.1.0", "FriendlyName": "uBot Sensor", "Description": "Simulated sensors.",
            "DocsURL": "https://example.com/docs",
            "UBot": {
                "Layer": "Capability",
                "Repository": "https://github.com/AstreoX/uBotSensor.git",
                "Requires": [ { "Name": "UBotCore", "Version": "^0.1.0" }, { "Name": "SomeOptional", "Version": ">=1.0", "Optional": true } ],
                "Provides": [ "Sensor.IMU" ], "Tags": [ "sensor" ]
            }
        }"#,
        )
        .unwrap();
        assert!(descriptor.issues.is_empty(), "{:?}", descriptor.issues);
        assert_eq!(descriptor.version_name.as_deref(), Some("0.1.0"));
        assert_eq!(descriptor.friendly_name.as_deref(), Some("uBot Sensor"));
        assert_eq!(
            descriptor.docs_url.as_deref(),
            Some("https://example.com/docs")
        );
        let block = descriptor.ubot.unwrap();
        assert_eq!(block.layer, Layer::Capability);
        assert_eq!(
            block.repository.as_deref(),
            Some("https://github.com/AstreoX/uBotSensor.git")
        );
        assert_eq!(block.provides, ["Sensor.IMU"]);
        assert_eq!(block.tags, ["sensor"]);
        assert_eq!(
            block.requires,
            [
                Dependency {
                    name: "UBotCore".into(),
                    version: "^0.1.0".into(),
                    optional: false
                },
                Dependency {
                    name: "SomeOptional".into(),
                    version: ">=1.0".into(),
                    optional: true
                },
            ]
        );
    }

    #[test]
    fn plain_and_malformed_blocks() {
        let plain = parse(r#"{ "FileVersion": 3, "VersionName": "1.0.0", "Category": "uBot", "EnabledByDefault": false }"#).unwrap();
        assert!(!plain.is_ubot());
        assert_eq!(plain.enabled_by_default, Some(false));
        assert!(plain.issues.is_empty());

        let not_object = parse(r#"{ "VersionName": "1.0.0", "UBot": "yes" }"#).unwrap();
        assert!(not_object.is_ubot());
        assert!(not_object
            .issues
            .iter()
            .any(|issue| issue.contains("not an object")));

        let bad_layer =
            parse(r#"{ "VersionName": "1.0.0", "UBot": { "Layer": "Middleware" } }"#).unwrap();
        assert_eq!(bad_layer.ubot.as_ref().unwrap().layer, Layer::Unknown);
        assert!(bad_layer
            .issues
            .iter()
            .any(|issue| issue.contains("Middleware")));

        let no_layer = parse(r#"{ "VersionName": "1.0.0", "UBot": {} }"#).unwrap();
        assert!(no_layer.issues.iter().any(|issue| issue.contains("Layer")));

        let object_layer =
            parse(r#"{ "VersionName": "1.0.0", "UBot": { "Layer": { "Name": "Adapter" } } }"#)
                .unwrap();
        assert!(object_layer
            .issues
            .iter()
            .any(|issue| issue.contains("not a string")));

        let lower = parse(r#"{ "VersionName": "1.0.0", "UBot": { "Layer": "adapter" } }"#).unwrap();
        assert_eq!(lower.ubot.unwrap().layer, Layer::Adapter);
        assert!(lower.issues.is_empty());

        let bad_version =
            parse(r#"{ "VersionName": "beta", "UBot": { "Layer": "Content" } }"#).unwrap();
        assert!(bad_version
            .issues
            .iter()
            .any(|issue| issue.contains("beta")));
        let no_version = parse(r#"{ "UBot": { "Layer": "Content" } }"#).unwrap();
        assert!(no_version
            .issues
            .iter()
            .any(|issue| issue.contains("VersionName")));
        assert!(parse("[]").is_err());
        assert!(parse("nope").is_err());
    }

    #[test]
    fn malformed_requirements_are_skipped() {
        let descriptor = parse(
            r#"{
            "VersionName": "1.0.0",
            "UBot": {
                "Layer": "Content",
                "Requires": [
                    { "Name": "A" },
                    { "Name": " B ", "Version": " >=1.0 ", "Optional": true },
                    { "Version": "^1.0" },
                    "C",
                    { "Name": "D", "Optional": [ true ] },
                    { "Name": "E", "Version": "~2.1", "Optional": false },
                    { "Name": "F", "Version": "1.x" }
                ],
                "Tags": [ "a", {}, "", "a" ],
                "ExternalRequires": [ "TempoROS" ],
                "EnginePlugins": [ "ChaosVehiclesPlugin", "temporos" ]
            }
        }"#,
        )
        .unwrap();
        let block = descriptor.ubot.as_ref().unwrap();
        let names: Vec<&str> = block.requires.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["A", "B", "E", "F"]);
        assert_eq!(block.requires[1].version, ">=1.0");
        assert!(block.requires[1].optional);
        let issues = descriptor.issues.join("\n");
        assert!(issues.contains("\"Requires\"[2]"));
        assert!(issues.contains("\"Requires\"[3]"));
        assert!(issues.contains("\"Requires\"[4]"));
        assert!(issues.contains("Requirement 'F' has an unparsable version constraint"));
        assert_eq!(block.tags, ["a"]);
        assert!(issues.contains("\"Tags\"[1]") && issues.contains("\"Tags\"[2]"));
        assert_eq!(block.engine_plugins, ["ChaosVehiclesPlugin", "temporos"]);

        let not_array = parse(
            r#"{ "VersionName": "1.0.0", "UBot": { "Layer": "Content", "Requires": "UBotCore" } }"#,
        )
        .unwrap();
        assert!(not_array.ubot.unwrap().requires.is_empty());
        assert!(not_array
            .issues
            .iter()
            .any(|issue| issue.contains("\"Requires\" is not an array")));
    }
}
