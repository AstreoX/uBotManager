//! The runtime status file `Saved/uBot/status.json` written by UBotCore (SPEC §9.2).

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use crate::model::{SessionKind, SessionStatus, Severity};
use crate::paths;

/// A problem a package reported during the last session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionProblem {
    pub id: String,
    pub package: String,
    pub severity: Severity,
    pub code: String,
    pub params: BTreeMap<String, String>,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionFile {
    pub session: SessionStatus,
    pub problems: Vec<SessionProblem>,
}

/// Reads the status file; `None` when it is missing or not understood.
pub fn load(path: &Path) -> Option<SessionFile> {
    let (text, _) = paths::read_text(path).ok()?;
    parse(&text).ok()
}

pub fn parse(text: &str) -> Result<SessionFile, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("invalid JSON: {error}"))?;
    if value.get("formatVersion").and_then(Value::as_u64) != Some(1) {
        return Err("unsupported formatVersion".into());
    }
    let session = value.get("session").ok_or("no \"session\"")?;
    let kind = match session.get("kind").and_then(Value::as_str) {
        Some("PIE") => SessionKind::Pie,
        Some("Editor") => SessionKind::Editor,
        Some("Game") => SessionKind::Game,
        other => return Err(format!("unknown session kind {other:?}")),
    };
    let text_field =
        |object: &Value, key: &str| object.get(key).and_then(Value::as_str).map(str::to_string);
    let status = SessionStatus {
        kind,
        started_at: text_field(session, "startedAt").ok_or("no \"session.startedAt\"")?,
        ended_at: text_field(session, "endedAt").filter(|ended| !ended.is_empty()),
        engine_version: text_field(&value, "engineVersion").unwrap_or_default(),
    };

    let problems = value
        .get("problems")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(parse_problem).collect())
        .unwrap_or_default();
    Ok(SessionFile {
        session: status,
        problems,
    })
}

fn parse_problem(item: &Value) -> Option<SessionProblem> {
    let text = |key: &str| item.get(key).and_then(Value::as_str).map(str::to_string);
    let id = text("id").filter(|id| !id.is_empty())?;
    let severity = match text("severity").as_deref() {
        Some("error") => Severity::Error,
        Some("warning") => Severity::Warning,
        _ => Severity::Info,
    };
    let params = item
        .get("params")
        .and_then(Value::as_object)
        .map(|params| {
            params
                .iter()
                .map(|(key, value)| {
                    let value = match value {
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    };
                    (key.clone(), value)
                })
                .collect()
        })
        .unwrap_or_default();
    Some(SessionProblem {
        package: text("package").unwrap_or_default(),
        severity,
        code: text("code").unwrap_or_default(),
        message: text("message").unwrap_or_else(|| id.clone()),
        params,
        id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_status_file() {
        let file = parse(
            r#"{
            "formatVersion": 1,
            "writtenAt": "2026-10-10T08:00:00Z",
            "session": { "kind": "PIE", "startedAt": "2026-10-10T07:50:00Z", "endedAt": "2026-10-10T08:00:00Z" },
            "engineVersion": "5.5.4",
            "packages": [ { "name": "UBotCore", "version": "0.1.0", "enabled": true } ],
            "runtime": [],
            "problems": [
                { "id": "ros.bridge", "package": "UBotROS", "severity": "info", "code": "rosBridgeUnreachable",
                  "params": { "address": "127.0.0.1:8268", "attempts": 3 }, "message": "Could not connect." },
                { "package": "NoId" }
            ]
        }"#,
        )
        .unwrap();
        assert_eq!(file.session.kind, SessionKind::Pie);
        assert_eq!(
            file.session.ended_at.as_deref(),
            Some("2026-10-10T08:00:00Z")
        );
        assert_eq!(file.session.engine_version, "5.5.4");
        assert_eq!(file.problems.len(), 1);
        let problem = &file.problems[0];
        assert_eq!(
            (
                problem.id.as_str(),
                problem.package.as_str(),
                problem.severity
            ),
            ("ros.bridge", "UBotROS", Severity::Info)
        );
        assert_eq!(problem.params["address"], "127.0.0.1:8268");
        assert_eq!(problem.params["attempts"], "3");

        assert!(parse(r#"{ "formatVersion": 2, "session": {} }"#).is_err());
        assert!(parse(
            r#"{ "formatVersion": 1, "session": { "kind": "Other", "startedAt": "x" } }"#
        )
        .is_err());
        assert!(parse("nope").is_err());
    }
}
