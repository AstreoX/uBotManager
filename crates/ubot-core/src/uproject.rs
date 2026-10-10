//! Reading and editing `.uproject` files while keeping their formatting (SPEC §3): key order and
//! unknown fields, other fields of existing plugin entries, indentation, line endings, a trailing
//! newline and a byte order mark. Writes are atomic.

use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::paths;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Format {
    pub indent: String,
    pub crlf: bool,
    pub trailing_newline: bool,
    pub bom: bool,
}

impl Default for Format {
    fn default() -> Self {
        Format {
            indent: "\t".into(),
            crlf: true,
            trailing_newline: false,
            bom: false,
        }
    }
}

impl Format {
    pub fn detect(text: &str, bom: bool) -> Format {
        let crlf = text.contains("\r\n");
        let trailing_newline = text.ends_with('\n');
        let indent = text
            .lines()
            .skip(1)
            .map(|line| line.trim_end_matches('\r'))
            .find_map(|line| {
                let whitespace: String = line
                    .chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .collect();
                (!whitespace.is_empty() && whitespace.len() < line.len()).then_some(whitespace)
            })
            .map(|whitespace| {
                if whitespace.starts_with('\t') {
                    "\t".to_string()
                } else {
                    whitespace
                }
            })
            .unwrap_or_else(|| "\t".to_string());
        Format {
            indent,
            crlf,
            trailing_newline,
            bom,
        }
    }
}

/// A plugin entry of the `.uproject` `Plugins` array.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginEntry {
    pub name: String,
    pub enabled: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct UProject {
    pub path: PathBuf,
    pub json: Map<String, Value>,
    pub format: Format,
}

impl UProject {
    pub fn load(path: &Path) -> io::Result<UProject> {
        let (text, bom) = paths::read_text(path)?;
        UProject::parse(path, &text, bom)
    }

    pub fn parse(path: &Path, text: &str, bom: bool) -> io::Result<UProject> {
        let value: Value = serde_json::from_str(text).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} is not valid JSON: {error}", path.display()),
            )
        })?;
        let Value::Object(json) = value else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} is not a JSON object", path.display()),
            ));
        };
        Ok(UProject {
            path: path.to_path_buf(),
            json,
            format: Format::detect(text, bom),
        })
    }

    pub fn engine_association(&self) -> String {
        self.json
            .get("EngineAssociation")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    pub fn additional_plugin_directories(&self) -> Vec<String> {
        self.json
            .get("AdditionalPluginDirectories")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn plugins(&self) -> Vec<PluginEntry> {
        self.json
            .get("Plugins")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let name = item.get("Name")?.as_str()?.to_string();
                        Some(PluginEntry {
                            name,
                            enabled: item.get("Enabled").and_then(Value::as_bool),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The `Enabled` value of the plugin's entry, if it has one.
    pub fn entry_enabled(&self, name: &str) -> Option<bool> {
        self.plugins()
            .into_iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(name))
            .and_then(|entry| entry.enabled)
    }

    fn plugins_mut(&mut self) -> &mut Vec<Value> {
        let plugins = self
            .json
            .entry("Plugins")
            .or_insert_with(|| Value::Array(Vec::new()));
        if !plugins.is_array() {
            *plugins = Value::Array(Vec::new());
        }
        plugins.as_array_mut().expect("just made an array")
    }

    /// Sets `"Enabled"` on the plugin's entry, appending `{ "Name", "Enabled" }` when there is none.
    /// Returns whether anything changed.
    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> bool {
        let plugins = self.plugins_mut();
        let existing = plugins.iter_mut().find(|item| {
            item.get("Name")
                .and_then(Value::as_str)
                .is_some_and(|entry| entry.eq_ignore_ascii_case(name))
        });
        match existing.and_then(Value::as_object_mut) {
            Some(entry) => {
                if entry.get("Enabled") == Some(&Value::Bool(enabled)) {
                    return false;
                }
                entry.insert("Enabled".into(), Value::Bool(enabled));
                true
            }
            None => {
                let mut entry = Map::new();
                entry.insert("Name".into(), Value::String(name.to_string()));
                entry.insert("Enabled".into(), Value::Bool(enabled));
                plugins.push(Value::Object(entry));
                true
            }
        }
    }

    /// Deletes the plugin's entry. Returns whether there was one.
    pub fn remove_entry(&mut self, name: &str) -> bool {
        let Some(plugins) = self.json.get_mut("Plugins").and_then(Value::as_array_mut) else {
            return false;
        };
        let before = plugins.len();
        plugins.retain(|item| {
            !item
                .get("Name")
                .and_then(Value::as_str)
                .is_some_and(|entry| entry.eq_ignore_ascii_case(name))
        });
        plugins.len() != before
    }

    /// The file contents in the detected format.
    pub fn render(&self) -> String {
        let mut out = Vec::new();
        let formatter =
            serde_json::ser::PrettyFormatter::with_indent(self.format.indent.as_bytes());
        let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
        self.json
            .serialize(&mut serializer)
            .expect("serializing to memory cannot fail");
        let mut text = String::from_utf8(out).expect("serde_json writes UTF-8");
        if self.format.trailing_newline {
            text.push('\n');
        }
        if self.format.crlf {
            // JSON strings cannot contain raw line breaks, so every '\n' is formatting.
            text = text.replace('\n', "\r\n");
        }
        if self.format.bom {
            text.insert(0, '\u{feff}');
        }
        text
    }

    pub fn save(&self) -> io::Result<()> {
        paths::write_atomic(&self.path, self.render().as_bytes())
    }
}

/// Loads the file, applies `edit` and saves it when `edit` reports a change.
pub fn edit(path: &Path, edit: impl FnOnce(&mut UProject) -> bool) -> io::Result<bool> {
    let mut project = UProject::load(path)?;
    if edit(&mut project) {
        project.save()?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABS_CRLF: &str = "{\r\n\t\"FileVersion\": 3,\r\n\t\"EngineAssociation\": \"5.5\",\r\n\t\"Category\": \"\",\r\n\t\"Description\": \"\",\r\n\t\"Plugins\": [\r\n\t\t{\r\n\t\t\t\"Name\": \"ModelingToolsEditorMode\",\r\n\t\t\t\"Enabled\": true,\r\n\t\t\t\"TargetAllowList\": [\r\n\t\t\t\t\"Editor\"\r\n\t\t\t]\r\n\t\t}\r\n\t],\r\n\t\"Custom\": { \"Keep\": [ 1, 2 ] }\r\n}";

    const SPACES_LF: &str = "{\n    \"FileVersion\": 3,\n    \"EngineAssociation\": \"5.5\",\n    \"AdditionalPluginDirectories\": [\n        \"../Plugins/uBot\"\n    ],\n    \"Plugins\": [\n        {\n            \"Name\": \"UBotCore\",\n            \"Enabled\": true\n        },\n        {\n            \"Name\": \"TempoROS\",\n            \"Enabled\": false\n        }\n    ]\n}\n";

    #[test]
    fn round_trips_without_changes() {
        let project = UProject::parse(Path::new("x.uproject"), SPACES_LF, false).unwrap();
        assert_eq!(
            project.format,
            Format {
                indent: "    ".into(),
                crlf: false,
                trailing_newline: true,
                bom: false
            }
        );
        assert_eq!(project.render(), SPACES_LF);
        assert_eq!(project.additional_plugin_directories(), ["../Plugins/uBot"]);
        assert_eq!(project.entry_enabled("tempoRos"), Some(false));
        assert_eq!(project.entry_enabled("Nope"), None);
        assert_eq!(project.engine_association(), "5.5");
    }

    #[test]
    fn edits_tab_indented_crlf_files() {
        let mut project = UProject::parse(Path::new("x.uproject"), TABS_CRLF, false).unwrap();
        assert_eq!(
            project.format,
            Format {
                indent: "\t".into(),
                crlf: true,
                trailing_newline: false,
                bom: false
            }
        );

        assert!(project.set_enabled("UBotCore", true));
        assert!(project.set_enabled("modelingtoolseditormode", false));
        assert!(!project.set_enabled("UBotCore", true));
        let text = project.render();
        assert!(
            !text.replace("\r\n", "").contains('\n'),
            "only CRLF line endings"
        );
        assert!(!text.ends_with('\n'));
        let expected = "{\r\n\t\"FileVersion\": 3,\r\n\t\"EngineAssociation\": \"5.5\",\r\n\t\"Category\": \"\",\r\n\t\"Description\": \"\",\r\n\t\"Plugins\": [\r\n\t\t{\r\n\t\t\t\"Name\": \"ModelingToolsEditorMode\",\r\n\t\t\t\"Enabled\": false,\r\n\t\t\t\"TargetAllowList\": [\r\n\t\t\t\t\"Editor\"\r\n\t\t\t]\r\n\t\t},\r\n\t\t{\r\n\t\t\t\"Name\": \"UBotCore\",\r\n\t\t\t\"Enabled\": true\r\n\t\t}\r\n\t],\r\n\t\"Custom\": {\r\n\t\t\"Keep\": [\r\n\t\t\t1,\r\n\t\t\t2\r\n\t\t]\r\n\t}\r\n}";
        assert_eq!(text, expected);

        assert!(project.remove_entry("ubotcore"));
        assert!(!project.remove_entry("ubotcore"));
        assert_eq!(project.plugins().len(), 1);
    }

    #[test]
    fn edits_space_indented_lf_files_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Dev.uproject");
        let mut bytes = b"\xEF\xBB\xBF".to_vec();
        bytes.extend_from_slice(SPACES_LF.as_bytes());
        std::fs::write(&path, bytes).unwrap();

        assert!(edit(&path, |project| project.set_enabled("UBotSensor", true)).unwrap());
        assert!(!edit(&path, |project| project.set_enabled("UBotSensor", true)).unwrap());
        let written = std::fs::read(&path).unwrap();
        assert!(written.starts_with(b"\xEF\xBB\xBF{\n    \"FileVersion\": 3,\n"));
        let text = String::from_utf8(written[3..].to_vec()).unwrap();
        assert!(text.ends_with("        {\n            \"Name\": \"UBotSensor\",\n            \"Enabled\": true\n        }\n    ]\n}\n"));
        assert!(!text.contains('\r'));

        assert!(edit(&path, |project| project.remove_entry("UBotSensor")).unwrap());
        let (text, bom) = paths::read_text(&path).unwrap();
        assert!(bom);
        assert_eq!(text, SPACES_LF);
    }

    #[test]
    fn creates_the_plugins_array() {
        let mut project =
            UProject::parse(Path::new("x.uproject"), "{\n  \"FileVersion\": 3\n}", false).unwrap();
        assert_eq!(project.format.indent, "  ");
        project.set_enabled("A", false);
        assert_eq!(project.render(), "{\n  \"FileVersion\": 3,\n  \"Plugins\": [\n    {\n      \"Name\": \"A\",\n      \"Enabled\": false\n    }\n  ]\n}");
        assert!(UProject::parse(Path::new("x"), "[]", false).is_err());
    }
}
