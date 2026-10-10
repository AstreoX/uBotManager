//! Package index, format 2 (SPEC §1): parsing, validation and merging of several sources.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{Layer, LocalizedText};
use crate::url::{
    check_binary_url, check_repository_url, is_safe_folder_name, redact_credentials,
    repository_folder_name, strip_url_secrets, UrlVerdict,
};
use crate::version::{Constraint, SemVer};

pub const FORMAT_VERSION: u64 = 2;

/// Check ids a recipe may list; others are ignored (and reported as warnings by validation).
pub const KNOWN_RECIPE_CHECKS: &[&str] = &["rosBridge"];

/// The platforms of `binaries` the manager installs; other platform keys are ignored.
pub const BINARY_PLATFORMS: &[&str] = &["Win64"];

/// A prebuilt archive is never larger than this (an exact `size` may set a lower limit).
pub const MAX_BINARY_BYTES: u64 = 2 * 1024 * 1024 * 1024;

const BINARY_KEYS: &[&str] = &["url", "sha256", "size"];

/// One `binaries[engine][platform]` entry of a version: a zip of the built plugin (SPEC §1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrebuiltBinary {
    /// `https://`, `file://` or an absolute local path.
    pub url: String,
    /// SHA-256 of the archive, 64 lower-case hex digits.
    pub sha256: String,
    /// The exact size of the archive in bytes, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// `Major.Minor`, the engine keys of `binaries` ("5.5").
pub fn is_engine_key(key: &str) -> bool {
    match key.split_once('.') {
        Some((major, minor)) => {
            !major.is_empty()
                && !minor.is_empty()
                && major.bytes().all(|b| b.is_ascii_digit())
                && minor.bytes().all(|b| b.is_ascii_digit())
        }
        None => false,
    }
}

/// 64 lower-case hex digits.
pub fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Reads one `binaries[engine][platform]` entry. `Err` lists what is wrong with it; the
/// unknown keys of a usable entry come back with it (validation warns about them).
pub fn parse_binary(value: &Value) -> Result<(PrebuiltBinary, Vec<String>), String> {
    let Some(object) = value.as_object() else {
        return Err("must be an object with \"url\" and \"sha256\"".into());
    };
    let mut problems = Vec::new();
    let url = match object.get("url") {
        Some(Value::String(url)) => {
            if let Err(reason) = check_binary_url(url) {
                problems.push(format!(
                    "url '{}' is not allowed: {reason}",
                    redact_credentials(url)
                ));
            }
            url.clone()
        }
        Some(_) => {
            problems.push("\"url\" is not a string".to_string());
            String::new()
        }
        None => {
            problems.push("\"url\" is missing".to_string());
            String::new()
        }
    };
    let sha256 = match object.get("sha256") {
        Some(Value::String(sha256)) if is_sha256_hex(sha256) => sha256.clone(),
        Some(_) => {
            problems.push("\"sha256\" must be 64 lower-case hex digits".to_string());
            String::new()
        }
        None => {
            problems.push("\"sha256\" is missing".to_string());
            String::new()
        }
    };
    let size = match object.get("size") {
        None | Some(Value::Null) => None,
        Some(value) => match value.as_u64() {
            Some(size) if size > 0 && size <= MAX_BINARY_BYTES => Some(size),
            Some(size) if size > MAX_BINARY_BYTES => {
                problems.push(format!(
                    "\"size\" {size} is over the limit of {MAX_BINARY_BYTES} bytes"
                ));
                None
            }
            _ => {
                problems.push("\"size\" must be a positive number of bytes".to_string());
                None
            }
        },
    };
    if !problems.is_empty() {
        return Err(problems.join("; "));
    }
    let unknown = object
        .keys()
        .filter(|key| !BINARY_KEYS.contains(&key.as_str()))
        .cloned()
        .collect();
    Ok((PrebuiltBinary { url, sha256, size }, unknown))
}

/// Checks a version's `binaries` (null is fine): errors and warnings, each prefixed with `at`
/// and the place inside `binaries`.
fn check_binaries(value: &Value, at: &str, errors: &mut Vec<String>, warnings: &mut Vec<String>) {
    let engines = match value {
        Value::Null => return,
        Value::Object(engines) => engines,
        _ => {
            errors.push(format!(
                "{at}.binaries: must be an object of engine versions (\"5.5\") or null."
            ));
            return;
        }
    };
    for (engine, platforms) in engines {
        let eat = format!("{at}.binaries[\"{engine}\"]");
        if !is_engine_key(engine) {
            errors.push(format!(
                "{eat}: '{engine}' is not an engine version like \"5.5\" (Major.Minor)."
            ));
            continue;
        }
        let Some(platforms) = platforms.as_object() else {
            errors.push(format!(
                "{eat}: must be an object of platforms (\"Win64\")."
            ));
            continue;
        };
        for (platform, entry) in platforms {
            let pat = format!("{eat}[\"{platform}\"]");
            if !BINARY_PLATFORMS.contains(&platform.as_str()) {
                warnings.push(format!(
                    "{pat}: unknown platform '{platform}' is ignored (only Win64 is installed)."
                ));
                continue;
            }
            match parse_binary(entry) {
                Ok((_, unknown)) => {
                    for key in unknown {
                        warnings.push(format!("{pat}: unknown key \"{key}\" is ignored."));
                    }
                }
                Err(problems) => errors.push(format!("{pat}: {problems}.")),
            }
        }
    }
}

/// Removes credentials, query and fragment from the `url`s of a `binaries` value: signed download
/// links keep their tokens in the query.
fn redact_binaries(value: &mut Value) {
    let Some(engines) = value.as_object_mut() else {
        return;
    };
    for platforms in engines.values_mut() {
        let Some(platforms) = platforms.as_object_mut() else {
            continue;
        };
        for entry in platforms.values_mut() {
            if let Some(Value::String(url)) = entry.get_mut("url") {
                *url = strip_url_secrets(url);
            }
        }
    }
}

/// Localized text in the index: "en" is required, any other locale is optional.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexText {
    pub en: String,
    #[serde(flatten)]
    pub other: BTreeMap<String, String>,
}

impl IndexText {
    pub fn zh_cn(&self) -> Option<&str> {
        self.other.get("zh-CN").map(String::as_str)
    }

    pub fn to_localized(&self) -> LocalizedText {
        LocalizedText {
            en: self.en.clone(),
            zh_cn: self.zh_cn().map(str::to_string),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexRequirement {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub optional: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexVersion {
    pub version: String,
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub planned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    #[serde(default)]
    pub requires: Vec<IndexRequirement>,
    #[serde(default)]
    pub engine_plugins: Vec<String>,
    #[serde(default)]
    pub provides: Vec<String>,
    /// Prebuilt binaries: `{ "<Major.Minor>": { "<platform>": PrebuiltBinary } }`. Kept as written
    /// (a malformed entry does not make the index unreadable; validation reports it, and
    /// [`IndexVersion::prebuilt`] only returns usable entries).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binaries: Option<Value>,
}

impl IndexVersion {
    pub fn semver(&self) -> Option<SemVer> {
        SemVer::parse(&self.version)
    }

    /// The prebuilt binaries of this version for an engine (`Major.Minor`) and platform, when the
    /// index has a usable entry for them (URL policy, SHA-256 and size checked).
    pub fn prebuilt(&self, engine: &str, platform: &str) -> Option<PrebuiltBinary> {
        if self.planned || !is_engine_key(engine) {
            return None;
        }
        let entry = self
            .binaries
            .as_ref()?
            .as_object()?
            .get(engine)?
            .as_object()?
            .get(platform)?;
        parse_binary(entry).ok().map(|(binary, _)| binary)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexPackage {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub friendly_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<IndexText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub versions: Vec<IndexVersion>,
}

impl IndexPackage {
    pub fn layer(&self) -> Layer {
        self.layer
            .as_deref()
            .and_then(Layer::parse)
            .unwrap_or(Layer::Unknown)
    }

    /// Non-planned versions that parse, newest first.
    pub fn released_versions(&self) -> Vec<(&IndexVersion, SemVer)> {
        let mut versions: Vec<(&IndexVersion, SemVer)> = self
            .versions
            .iter()
            .filter(|v| !v.planned)
            .filter_map(|v| v.semver().map(|semver| (v, semver)))
            .collect();
        versions.sort_by(|a, b| b.1.cmp(&a.1));
        versions
    }

    /// Highest non-planned version.
    pub fn latest(&self) -> Option<&IndexVersion> {
        self.released_versions()
            .first()
            .map(|(version, _)| *version)
    }

    /// A package with no non-planned version is planned.
    pub fn is_planned(&self) -> bool {
        self.latest().is_none()
    }

    /// Highest planned version above the latest release (any planned version when nothing is released).
    pub fn planned_version(&self) -> Option<&IndexVersion> {
        let latest = self.latest().and_then(IndexVersion::semver);
        self.versions
            .iter()
            .filter(|v| v.planned)
            .filter_map(|v| v.semver().map(|semver| (v, semver)))
            .filter(|(_, semver)| latest.as_ref().is_none_or(|latest| semver > latest))
            .max_by(|a, b| a.1.cmp(&b.1))
            .map(|(version, _)| version)
    }

    /// The version whose requirements describe the package when it is not installed: the latest
    /// release, else the highest planned version.
    pub fn effective_version(&self) -> Option<&IndexVersion> {
        self.latest().or_else(|| {
            self.versions
                .iter()
                .filter_map(|v| v.semver().map(|semver| (v, semver)))
                .max_by(|a, b| a.1.cmp(&b.1))
                .map(|(version, _)| version)
        })
    }

    /// The non-planned version equal to `version` (SemVer precedence).
    pub fn find_version(&self, version: &str) -> Option<&IndexVersion> {
        let wanted = SemVer::parse(version)?;
        self.versions
            .iter()
            .filter(|v| !v.planned)
            .find(|v| v.semver().is_some_and(|semver| semver == wanted))
    }

    /// Install folder: explicit, else derived from the repository.
    pub fn folder(&self) -> Option<String> {
        match &self.folder {
            Some(folder) => Some(folder.clone()),
            None => self.repository.as_deref().and_then(repository_folder_name),
        }
    }

    pub fn friendly_name(&self) -> &str {
        self.friendly_name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(&self.name)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexRecipe {
    pub id: String,
    pub name: IndexText,
    #[serde(default)]
    pub description: IndexText,
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub checks: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Replacement {
    pub legacy: String,
    pub replacement: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirects: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexFile {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub format_version: u64,
    #[serde(default)]
    pub name: String,
    pub packages: Vec<IndexPackage>,
    #[serde(default)]
    pub recipes: Vec<IndexRecipe>,
    #[serde(default)]
    pub replacements: Vec<Replacement>,
}

impl IndexFile {
    pub fn find(&self, name: &str) -> Option<&IndexPackage> {
        self.packages
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name.trim()))
    }

    pub fn to_json(&self) -> String {
        to_pretty_json(self)
    }

    /// A copy whose URLs have their credentials removed (`https://user:token@host` becomes
    /// `https://***@host`; binaries URLs also lose their query), for output that is shown or
    /// logged.
    pub fn redacted(&self) -> IndexFile {
        let mut file = self.clone();
        for package in &mut file.packages {
            for url in [&mut package.repository, &mut package.docs_url]
                .into_iter()
                .flatten()
            {
                *url = redact_credentials(url);
            }
            for version in &mut package.versions {
                if let Some(binaries) = &mut version.binaries {
                    redact_binaries(binaries);
                }
            }
        }
        file
    }
}

impl IndexFile {
    /// A copy for the file the UE panel reads (`Saved/uBot/index.json`, SPEC §1.1). Repository
    /// and docs URLs are kept as written, because UE clones with them; `binaries` URLs are
    /// redacted, because UE ignores `binaries` and the file lives in the project folder.
    pub fn for_panel(&self) -> IndexFile {
        let mut file = self.clone();
        for package in &mut file.packages {
            for version in &mut package.versions {
                if let Some(binaries) = &mut version.binaries {
                    redact_binaries(binaries);
                }
            }
        }
        file
    }
}

pub(crate) fn to_pretty_json<T: Serialize>(value: &T) -> String {
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    value
        .serialize(&mut serializer)
        .expect("serializing to memory cannot fail");
    out.push(b'\n');
    String::from_utf8(out).expect("serde_json writes UTF-8")
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Parses an index file. Errors name the offending entry ("packages[2] (UBotROS): …").
pub fn parse(text: &str) -> Result<IndexFile, String> {
    parse_with_value(text).map(|(index, _)| index)
}

/// Like [`parse`], also returning the raw JSON for validation of unknown keys.
pub fn parse_with_value(text: &str) -> Result<(IndexFile, Value), String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("Invalid JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or("The index must be a JSON object.")?;

    match object.get("formatVersion") {
        None => {
            let hint = if object.contains_key("FormatVersion") {
                " (format 1 indexes are no longer supported)"
            } else {
                ""
            };
            return Err(format!("The index has no \"formatVersion\"{hint}."));
        }
        Some(Value::Number(number)) if number.as_u64() == Some(FORMAT_VERSION) => {}
        Some(other) => {
            return Err(format!(
                "Unsupported formatVersion {other}; this reader supports {FORMAT_VERSION}."
            ))
        }
    }

    let packages = match object.get("packages") {
        Some(Value::Array(items)) => items,
        Some(_) => return Err("\"packages\" must be an array.".into()),
        None => return Err("The index has no \"packages\" array.".into()),
    };
    let mut parsed_packages = Vec::with_capacity(packages.len());
    for (position, item) in packages.iter().enumerate() {
        let label = item
            .get("name")
            .and_then(Value::as_str)
            .map(|name| format!(" ({name})"))
            .unwrap_or_default();
        let package: IndexPackage = serde_json::from_value(item.clone())
            .map_err(|error| format!("packages[{position}]{label}: {error}"))?;
        parsed_packages.push(package);
    }

    let recipes = parse_array::<IndexRecipe>(object.get("recipes"), "recipes", "id")?;
    let replacements =
        parse_array::<Replacement>(object.get("replacements"), "replacements", "legacy")?;
    let name = match object.get("name") {
        None => String::new(),
        Some(Value::String(name)) => name.clone(),
        Some(_) => return Err("\"name\" must be a string.".into()),
    };
    let schema = object
        .get("$schema")
        .and_then(Value::as_str)
        .map(str::to_string);

    Ok((
        IndexFile {
            schema,
            format_version: FORMAT_VERSION,
            name,
            packages: parsed_packages,
            recipes,
            replacements,
        },
        value,
    ))
}

fn parse_array<T: for<'de> Deserialize<'de>>(
    value: Option<&Value>,
    field: &str,
    label_key: &str,
) -> Result<Vec<T>, String> {
    let items = match value {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(format!("\"{field}\" must be an array.")),
    };
    items
        .iter()
        .enumerate()
        .map(|(position, item)| {
            let label = item
                .get(label_key)
                .and_then(Value::as_str)
                .map(|name| format!(" ({name})"))
                .unwrap_or_default();
            serde_json::from_value(item.clone())
                .map_err(|error| format!("{field}[{position}]{label}: {error}"))
        })
        .collect()
}

/// Result of [`validate`]: errors make CI fail, warnings do not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Validation {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl Validation {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

const TOP_KEYS: &[&str] = &[
    "$schema",
    "formatVersion",
    "name",
    "packages",
    "recipes",
    "replacements",
];
const PACKAGE_KEYS: &[&str] = &[
    "name",
    "friendlyName",
    "description",
    "layer",
    "repository",
    "folder",
    "docsUrl",
    "tags",
    "versions",
];
const VERSION_KEYS: &[&str] = &[
    "version",
    "ref",
    "planned",
    "engine",
    "requires",
    "enginePlugins",
    "provides",
    "binaries",
];
const REQUIRE_KEYS: &[&str] = &["name", "version", "optional"];
const RECIPE_KEYS: &[&str] = &["id", "name", "description", "packages", "checks"];
const REPLACEMENT_KEYS: &[&str] = &["legacy", "replacement", "redirects"];

/// Parses and validates index text (SPEC §1). A file that does not parse yields one error.
pub fn validate_text(text: &str) -> Validation {
    match parse_with_value(text) {
        Ok((index, value)) => validate(&index, Some(&value)),
        Err(error) => Validation {
            errors: vec![error],
            warnings: vec![],
        },
    }
}

/// Semantic validation. `raw` (the parsed JSON) enables warnings about unknown keys.
pub fn validate(index: &IndexFile, raw: Option<&Value>) -> Validation {
    let mut report = Validation::default();
    let errors = &mut report.errors;
    let warnings = &mut report.warnings;

    if index.format_version != FORMAT_VERSION {
        errors.push(format!("formatVersion must be {FORMAT_VERSION}."));
    }
    if index.name.trim().is_empty() {
        warnings.push("The index has no \"name\".".into());
    }

    let mut names: HashMap<String, usize> = HashMap::new();
    let mut folders: HashMap<String, String> = HashMap::new();
    for (position, package) in index.packages.iter().enumerate() {
        let at = format!("packages[{position}] ({})", package.name);
        let name = package.name.trim();
        if name.is_empty() {
            errors.push(format!("packages[{position}]: \"name\" is empty."));
        } else if !is_plugin_name(name) {
            errors.push(format!("{at}: \"{name}\" is not a valid plugin name (letters, digits and '_', not starting with a digit)."));
        }
        if let Some(first) = names.insert(name.to_ascii_lowercase(), position) {
            errors.push(format!(
                "{at}: the name is already used by packages[{first}] (names are case-insensitive)."
            ));
            continue;
        }

        match package.layer.as_deref() {
            None => warnings.push(format!("{at}: no \"layer\".")),
            Some(layer) if Layer::parse(layer).is_none_or(|parsed| parsed == Layer::Unknown) => {
                errors.push(format!("{at}: unknown layer '{layer}' (expected Foundation, Capability, Adapter or Content)."))
            }
            Some(_) => {}
        }
        match &package.description {
            None => warnings.push(format!("{at}: no \"description\".")),
            Some(text) => check_text(text, &format!("{at}.description"), errors, warnings),
        }

        let all_planned = package.versions.iter().all(|v| v.planned);
        match package.repository.as_deref() {
            None if !all_planned => errors.push(format!(
                "{at}: \"repository\" is required unless every version is planned."
            )),
            None => {}
            Some(repository) => match check_repository_url(repository) {
                UrlVerdict::Allowed => {}
                UrlVerdict::Insecure => {
                    warnings.push(format!("{at}: the repository uses plain http."))
                }
                UrlVerdict::Rejected(reason) => errors.push(format!(
                    "{at}: repository '{}' is not allowed: {reason}.",
                    crate::url::redact_credentials(repository)
                )),
            },
        }
        match package.folder() {
            Some(folder) if !is_safe_folder_name(&folder) => {
                errors.push(format!("{at}: folder '{folder}' must be one path segment of letters, digits, '.', '_' or '-'."))
            }
            Some(folder) => {
                if let Some(other) = folders.insert(folder.to_ascii_lowercase(), package.name.clone()) {
                    warnings.push(format!("{at}: installs into the same folder '{folder}' as {other}."));
                }
            }
            None if package.repository.is_some() => {
                errors.push(format!("{at}: no install folder can be derived from the repository; set \"folder\"."))
            }
            None => {}
        }

        if package.versions.is_empty() {
            errors.push(format!("{at}: \"versions\" is empty."));
        }
        let mut seen_versions: Vec<SemVer> = Vec::new();
        for (version_position, version) in package.versions.iter().enumerate() {
            let vat = format!("{at}.versions[{version_position}]");
            if !is_safe_version_text(&version.version) {
                errors.push(format!(
                    "{vat}: version '{}' may only contain letters, digits, '.', '-' and '+'.",
                    version.version
                ));
            }
            match version.semver() {
                None => errors.push(format!("{vat}: '{}' is not a version.", version.version)),
                Some(semver) => {
                    if seen_versions.contains(&semver) {
                        errors.push(format!(
                            "{vat}: version {} is listed twice.",
                            version.version
                        ));
                    }
                    seen_versions.push(semver);
                }
            }
            match (&version.git_ref, version.planned) {
                (None, false) => errors.push(format!(
                    "{vat}: \"ref\" is required for a version that is not planned."
                )),
                (Some(git_ref), false) if !is_safe_ref(git_ref) => {
                    errors.push(format!("{vat}: ref '{git_ref}' is not a safe git ref."))
                }
                (Some(_), true) => {
                    warnings.push(format!("{vat}: planned versions ignore \"ref\"."))
                }
                _ => {}
            }
            if let Some(engine) = &version.engine {
                if let Err(error) = Constraint::parse(engine) {
                    errors.push(format!("{vat}.engine: {error}."));
                }
            }
            for (require_position, requirement) in version.requires.iter().enumerate() {
                let rat = format!("{vat}.requires[{require_position}]");
                if requirement.name.trim().is_empty() {
                    errors.push(format!("{rat}: \"name\" is empty."));
                } else if index.find(&requirement.name).is_none() {
                    errors.push(format!(
                        "{rat}: '{}' is not in this index.",
                        requirement.name
                    ));
                }
                if let Err(error) = Constraint::parse(&requirement.version) {
                    errors.push(format!("{rat}: {error}."));
                }
            }
            for plugin in &version.engine_plugins {
                if !is_plugin_name(plugin.trim()) {
                    errors.push(format!(
                        "{vat}.enginePlugins: '{plugin}' is not a valid plugin name."
                    ));
                }
            }
            if let Some(binaries) = &version.binaries {
                if version.planned && !binaries.is_null() {
                    warnings.push(format!("{vat}: planned versions ignore \"binaries\"."));
                }
                check_binaries(binaries, &vat, errors, warnings);
            }
        }
    }

    if let Some(cycle) = find_cycle(index) {
        errors.push(format!("Dependency cycle: {}.", cycle.join(" -> ")));
    }

    let mut recipe_ids = HashSet::new();
    for (position, recipe) in index.recipes.iter().enumerate() {
        let at = format!("recipes[{position}] ({})", recipe.id);
        if recipe.id.trim().is_empty() {
            errors.push(format!("recipes[{position}]: \"id\" is empty."));
        } else if !recipe_ids.insert(recipe.id.to_ascii_lowercase()) {
            errors.push(format!("{at}: the id is used twice."));
        }
        check_text(&recipe.name, &format!("{at}.name"), errors, warnings);
        check_text(
            &recipe.description,
            &format!("{at}.description"),
            errors,
            warnings,
        );
        if recipe.packages.is_empty() {
            errors.push(format!("{at}: \"packages\" is empty."));
        }
        for package in &recipe.packages {
            if index.find(package).is_none() {
                errors.push(format!("{at}: package '{package}' is not in this index."));
            }
        }
        for check in &recipe.checks {
            if !KNOWN_RECIPE_CHECKS.contains(&check.as_str()) {
                warnings.push(format!("{at}: unknown check '{check}' is ignored."));
            }
        }
    }

    let mut legacy_names = HashSet::new();
    for (position, replacement) in index.replacements.iter().enumerate() {
        let at = format!("replacements[{position}] ({})", replacement.legacy);
        if replacement.legacy.trim().is_empty() {
            errors.push(format!("replacements[{position}]: \"legacy\" is empty."));
        } else if !legacy_names.insert(replacement.legacy.to_ascii_lowercase()) {
            errors.push(format!("{at}: the legacy plugin is listed twice."));
        }
        if index.find(&replacement.replacement).is_none() {
            errors.push(format!(
                "{at}: replacement '{}' is not in this index.",
                replacement.replacement
            ));
        }
        if let Some(redirects) = &replacement.redirects {
            if redirects.contains("..")
                || crate::url::is_absolute_local_path(redirects)
                || redirects.contains('\\')
            {
                errors.push(format!(
                    "{at}: redirects must be a relative '/' path inside the replacement plugin."
                ));
            }
        }
    }

    if let Some(raw) = raw {
        check_unknown_keys(raw, warnings);
    }
    report
}

fn check_text(text: &IndexText, at: &str, errors: &mut Vec<String>, warnings: &mut Vec<String>) {
    if text.en.trim().is_empty() {
        errors.push(format!("{at}: \"en\" is empty."));
    }
    if text.zh_cn().is_none_or(|zh| zh.trim().is_empty()) {
        warnings.push(format!("{at}: no zh-CN text."));
    }
}

/// UE plugin names: letters, digits and '_', not starting with a digit.
pub fn is_plugin_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A ref that git cannot read as an option and that has no whitespace, control characters or
/// quotes (a quote would break out of the quoted process argument). Mirrors
/// `FUBotPackageIndex::IsSafeGitRef`; the same vectors are tested on both sides.
pub fn is_safe_ref(git_ref: &str) -> bool {
    !git_ref.is_empty()
        && !git_ref.starts_with('-')
        && !git_ref.contains("..")
        && git_ref.chars().all(|c| {
            !c.is_whitespace()
                && !c.is_control()
                && !matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\' | '"')
        })
}

/// A version string that is safe where it becomes part of a file name or a URL (an archive name,
/// a download URL template): letters, digits, `.`, `-` and `+` only. SemVer parsing ignores
/// everything after `+`, so this is what keeps `1.0.0+x/../../y` out. Same rule as the `version`
/// pattern of `index.schema.json`.
pub fn is_safe_version_text(version: &str) -> bool {
    !version.is_empty()
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
}

/// A dependency cycle among the packages, using the latest (or planned) version of each.
fn find_cycle(index: &IndexFile) -> Option<Vec<String>> {
    let names: Vec<&str> = index.packages.iter().map(|p| p.name.as_str()).collect();
    let position = |name: &str| {
        names
            .iter()
            .position(|candidate| candidate.eq_ignore_ascii_case(name.trim()))
    };
    let edges: Vec<Vec<usize>> = index
        .packages
        .iter()
        .map(|package| {
            package
                .effective_version()
                .map(|version| {
                    version
                        .requires
                        .iter()
                        .filter(|r| !r.optional)
                        .filter_map(|r| position(&r.name))
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();
    crate::graph::find_cycles(&edges)
        .into_iter()
        .next()
        .map(|cycle| {
            cycle
                .into_iter()
                .map(|node| names[node].to_string())
                .collect()
        })
}

fn check_unknown_keys(raw: &Value, warnings: &mut Vec<String>) {
    let report = |value: &Value, allowed: &[&str], at: &str, warnings: &mut Vec<String>| {
        if let Some(object) = value.as_object() {
            for key in object.keys() {
                if !allowed.contains(&key.as_str()) {
                    warnings.push(format!("{at}: unknown key \"{key}\" is ignored."));
                }
            }
        }
    };
    report(raw, TOP_KEYS, "index", warnings);
    let array = |value: &Value, key: &str| {
        value
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    for (position, package) in array(raw, "packages").iter().enumerate() {
        let at = format!("packages[{position}]");
        report(package, PACKAGE_KEYS, &at, warnings);
        for (version_position, version) in array(package, "versions").iter().enumerate() {
            let vat = format!("{at}.versions[{version_position}]");
            report(version, VERSION_KEYS, &vat, warnings);
            for (require_position, requirement) in array(version, "requires").iter().enumerate() {
                report(
                    requirement,
                    REQUIRE_KEYS,
                    &format!("{vat}.requires[{require_position}]"),
                    warnings,
                );
            }
        }
    }
    for (position, recipe) in array(raw, "recipes").iter().enumerate() {
        report(
            recipe,
            RECIPE_KEYS,
            &format!("recipes[{position}]"),
            warnings,
        );
    }
    for (position, replacement) in array(raw, "replacements").iter().enumerate() {
        report(
            replacement,
            REPLACEMENT_KEYS,
            &format!("replacements[{position}]"),
            warnings,
        );
    }
}

/// One loaded index source, in merge order.
#[derive(Clone, Debug)]
pub struct SourceIndex {
    pub id: String,
    pub index: IndexFile,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MergedPackage {
    pub package: IndexPackage,
    /// Id of the source that defined it.
    pub source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MergedRecipe {
    pub recipe: IndexRecipe,
    pub source: String,
}

/// The merged view of every loaded source (SPEC §1.1).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MergedIndex {
    pub packages: Vec<MergedPackage>,
    pub recipes: Vec<MergedRecipe>,
    pub replacements: Vec<Replacement>,
}

impl MergedIndex {
    /// The first source that defines a package (case-insensitive) wins for the whole package;
    /// recipes merge by id the same way; replacements are the union, first wins per legacy name.
    pub fn merge(sources: &[SourceIndex]) -> MergedIndex {
        let mut merged = MergedIndex::default();
        let mut package_names = HashSet::new();
        let mut recipe_ids = HashSet::new();
        let mut legacy_names = HashSet::new();
        for source in sources {
            for package in &source.index.packages {
                if package_names.insert(package.name.trim().to_ascii_lowercase()) {
                    merged.packages.push(MergedPackage {
                        package: package.clone(),
                        source: source.id.clone(),
                    });
                }
            }
            for recipe in &source.index.recipes {
                if recipe_ids.insert(recipe.id.trim().to_ascii_lowercase()) {
                    merged.recipes.push(MergedRecipe {
                        recipe: recipe.clone(),
                        source: source.id.clone(),
                    });
                }
            }
            for replacement in &source.index.replacements {
                if legacy_names.insert(replacement.legacy.trim().to_ascii_lowercase()) {
                    merged.replacements.push(replacement.clone());
                }
            }
        }
        merged
    }

    pub fn find(&self, name: &str) -> Option<&MergedPackage> {
        let name = name.trim();
        self.packages
            .iter()
            .find(|entry| entry.package.name.eq_ignore_ascii_case(name))
    }

    pub fn package(&self, name: &str) -> Option<&IndexPackage> {
        self.find(name).map(|entry| &entry.package)
    }

    /// Available = present and not planned.
    pub fn is_available(&self, name: &str) -> bool {
        self.package(name)
            .is_some_and(|package| !package.is_planned())
    }

    /// The merged index as a format 2 file named "merged".
    pub fn to_file(&self) -> IndexFile {
        IndexFile {
            schema: None,
            format_version: FORMAT_VERSION,
            name: "merged".into(),
            packages: self
                .packages
                .iter()
                .map(|entry| entry.package.clone())
                .collect(),
            recipes: self
                .recipes
                .iter()
                .map(|entry| entry.recipe.clone())
                .collect(),
            replacements: self.replacements.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "formatVersion": 2,
        "name": "test",
        "packages": [
            { "name": "Core", "description": { "en": "c", "zh-CN": "核" }, "layer": "Foundation",
              "repository": "https://example.com/core.git",
              "versions": [ { "version": "0.1.0", "ref": "v0.1.0" }, { "version": "0.2.0", "ref": "v0.2.0" },
                            { "version": "0.3.0", "planned": true } ] },
            { "name": "Sensor", "description": { "en": "s", "zh-CN": "传" }, "layer": "Capability",
              "repository": "https://example.com/uBotSensor.git",
              "versions": [ { "version": "0.1.0", "ref": "v0.1.0", "requires": [ { "name": "core", "version": "^0.1.0" } ] } ] },
            { "name": "Future", "description": { "en": "f", "zh-CN": "未" }, "layer": "Adapter",
              "versions": [ { "version": "0.1.0", "planned": true, "requires": [ { "name": "Core", "version": "^0.1.0" } ] } ] }
        ],
        "recipes": [ { "id": "r", "name": { "en": "R", "zh-CN": "R" }, "description": { "en": "D", "zh-CN": "D" },
                       "packages": [ "Core", "Sensor" ], "checks": [ "rosBridge" ] } ],
        "replacements": [ { "legacy": "Old", "replacement": "Sensor", "redirects": "Config/DefaultSensor.ini" } ]
    }"#;

    #[test]
    fn parses_and_derives_versions() {
        let index = parse(SAMPLE).unwrap();
        assert_eq!(index.packages.len(), 3);
        let core = index.find("core").unwrap();
        assert_eq!(core.latest().unwrap().version, "0.2.0");
        assert_eq!(core.planned_version().unwrap().version, "0.3.0");
        assert!(!core.is_planned());
        assert_eq!(core.folder().as_deref(), Some("core"));
        assert_eq!(
            core.released_versions()
                .iter()
                .map(|(v, _)| v.version.as_str())
                .collect::<Vec<_>>(),
            ["0.2.0", "0.1.0"]
        );
        assert!(core.find_version("0.1").is_some());
        assert!(
            core.find_version("0.3.0").is_none(),
            "planned versions are not installable"
        );

        let future = index.find("Future").unwrap();
        assert!(future.is_planned());
        assert_eq!(future.effective_version().unwrap().version, "0.1.0");
        assert_eq!(future.planned_version().unwrap().version, "0.1.0");
        assert_eq!(future.description.as_ref().unwrap().zh_cn(), Some("未"));

        let validation = validate_text(SAMPLE);
        assert!(validation.errors.is_empty(), "{:?}", validation.errors);
        assert!(validation.warnings.is_empty(), "{:?}", validation.warnings);
    }

    #[test]
    fn redacted_copies_hide_credentials() {
        let index = parse(
            r#"{ "formatVersion": 2, "packages": [
                { "name": "Sec", "repository": "https://user:SECRET123@h/team/Sec.git",
                  "docsUrl": "https://u:SECRET123@h/docs",
                  "versions": [ { "version": "0.1.0", "ref": "v0.1.0" } ] } ] }"#,
        )
        .unwrap();
        let redacted = index.redacted();
        assert_eq!(
            redacted.packages[0].repository.as_deref(),
            Some("https://***@h/team/Sec.git")
        );
        assert!(!redacted.to_json().contains("SECRET123"));
        assert!(
            index.to_json().contains("SECRET123"),
            "the original is untouched"
        );
    }

    #[test]
    fn parse_errors_are_specific() {
        assert!(parse("not json").unwrap_err().starts_with("Invalid JSON"));
        assert!(parse("[]").unwrap_err().contains("object"));
        assert!(parse(r#"{ "FormatVersion": 1, "Packages": [] }"#)
            .unwrap_err()
            .contains("format 1"));
        assert!(parse(r#"{ "formatVersion": 3, "packages": [] }"#)
            .unwrap_err()
            .contains("Unsupported formatVersion 3"));
        assert!(parse(r#"{ "formatVersion": 2 }"#)
            .unwrap_err()
            .contains("packages"));
        let error = parse(r#"{ "formatVersion": 2, "packages": [ { "name": "A", "versions": [] }, { "name": "B" } ] }"#).unwrap_err();
        assert!(
            error.starts_with("packages[1] (B): missing field `versions`"),
            "{error}"
        );
        let error =
            parse(r#"{ "formatVersion": 2, "packages": [], "recipes": [ { "id": "x" } ] }"#)
                .unwrap_err();
        assert!(error.starts_with("recipes[0] (x):"), "{error}");
        assert!(
            parse("\u{feff}{ \"formatVersion\": 2, \"packages\": [] }").is_ok(),
            "a BOM is accepted"
        );
    }

    #[test]
    fn validation_reports_errors_and_warnings() {
        let text = r#"{
            "formatVersion": 2,
            "packages": [
                { "name": "A", "layer": "Middle", "repository": "ext::sh", "versions": [ { "version": "1.0.0" }, { "version": "1.0" , "ref": "x" } ] },
                { "name": "a", "versions": [ { "version": "1.0.0", "ref": "v1" } ] },
                { "name": "B", "description": { "en": "b" }, "layer": "Content", "repository": "http://h/b.git", "folder": "../x",
                  "versions": [ { "version": "nope", "ref": "--upload-pack=x", "engine": ">=abc",
                                  "requires": [ { "name": "Ghost", "version": "^1" }, { "name": "C", "version": "1.x" } ] } ] },
                { "name": "C", "description": { "en": "c", "zh-CN": "c" }, "layer": "Adapter", "repository": "https://h/c.git",
                  "versions": [ { "version": "1.0.0", "ref": "v1", "requires": [ { "name": "D" } ] } ], "extra": 1 },
                { "name": "D", "description": { "en": "d", "zh-CN": "d" }, "layer": "Adapter", "repository": "https://h/d.git",
                  "versions": [ { "version": "1.0.0", "ref": "v1", "requires": [ { "name": "C" } ] } ] },
                { "name": "E", "description": { "en": "e", "zh-CN": "e" }, "layer": "Adapter",
                  "versions": [ { "version": "1.0.0", "ref": "v1" } ] }
            ],
            "recipes": [ { "id": "r", "name": { "en": "R" }, "packages": [ "Nobody" ], "checks": [ "warp" ] },
                         { "id": "R", "name": { "en": "R", "zh-CN": "R" }, "packages": [ "A" ] } ],
            "replacements": [ { "legacy": "Old", "replacement": "Missing" } ]
        }"#;
        let validation = validate_text(text);
        let errors = validation.errors.join("\n");
        let warnings = validation.warnings.join("\n");
        for fragment in [
            "unknown layer 'Middle'",
            "repository 'ext::sh' is not allowed",
            "versions[0]: \"ref\" is required",
            "version 1.0 is listed twice",
            "packages[1] (a): the name is already used by packages[0]",
            "folder '../x' must be one path segment",
            "'nope' is not a version",
            "ref '--upload-pack=x' is not a safe git ref",
            "engine: Invalid version 'abc'",
            "'Ghost' is not in this index",
            "Invalid version '1.x'",
            "Dependency cycle: C -> D -> C.",
            "\"repository\" is required unless every version is planned",
            "recipes[1] (R): the id is used twice",
            "package 'Nobody' is not in this index",
            "replacement 'Missing' is not in this index",
        ] {
            assert!(
                errors.contains(fragment),
                "missing error '{fragment}' in:\n{errors}"
            );
        }
        for fragment in [
            "packages[0] (A): no \"description\"",
            "packages[2] (B).description: no zh-CN text",
            "plain http",
            "unknown check 'warp'",
            "packages[3]: unknown key \"extra\"",
            "The index has no \"name\"",
        ] {
            assert!(
                warnings.contains(fragment),
                "missing warning '{fragment}' in:\n{warnings}"
            );
        }
    }

    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn with_binaries(binaries: &str) -> String {
        format!(
            r#"{{ "formatVersion": 2, "name": "t", "packages": [
                {{ "name": "A", "description": {{ "en": "a", "zh-CN": "a" }}, "layer": "Content",
                  "repository": "https://h/a.git",
                  "versions": [ {{ "version": "1.0.0", "ref": "v1", "binaries": {binaries} }} ] }} ] }}"#
        )
    }

    #[test]
    fn version_text_is_safe_in_file_names() {
        for good in ["0.1.0", "v1.2.3-rc.1+build.5", "1", "2.0.0+20260101"] {
            assert!(is_safe_version_text(good), "{good}");
        }
        for bad in [
            "",
            " 0.1.0 ",
            "1.0.0+x/../../pwn",
            "1.0.0+x\\y",
            "1.0.0 beta",
            "1.0.0:x",
            "1.0.0%2f",
            "1.0.0\u{e9}",
        ] {
            assert!(!is_safe_version_text(bad), "{bad:?}");
        }
        let one_version = |version: &str| {
            format!(
                r#"{{ "formatVersion": 2, "name": "t", "packages": [
                    {{ "name": "A", "description": {{ "en": "a", "zh-CN": "a" }}, "layer": "Content",
                      "repository": "https://h/a.git",
                      "versions": [ {{ "version": "{version}", "ref": "v1" }} ] }} ] }}"#
            )
        };
        for version in ["1.0.0+x/../../pwn", " 0.1.0 "] {
            let errors = validate_text(&one_version(version)).errors.join("\n");
            assert!(
                errors.contains("may only contain letters, digits"),
                "{version:?}: {errors}"
            );
        }
        for good in ["0.1.0", "v1.2.3-rc.1+build.5"] {
            let validation = validate_text(&one_version(good));
            assert!(
                validation.errors.is_empty(),
                "{good}: {:?}",
                validation.errors
            );
        }
    }

    #[test]
    fn the_panel_copy_keeps_repository_credentials_but_not_binaries_secrets() {
        let text = with_binaries(&format!(
            r#"{{ "5.5": {{ "Win64": {{ "url": "https://u:SECRETPW@h/x.zip?sig=SECRETSIG", "sha256": "{SHA}" }} }} }}"#
        ))
        .replace("https://h/a.git", "https://user:REPOTOKEN@h/a.git");
        let index = parse(&text).unwrap();
        let panel = index.for_panel().to_json();
        assert!(
            !panel.contains("SECRETPW") && !panel.contains("SECRETSIG"),
            "{panel}"
        );
        assert!(panel.contains("https://***@h/x.zip"), "{panel}");
        assert!(panel.contains("REPOTOKEN"), "UE clones with these: {panel}");
        let shown = index.redacted().to_json();
        assert!(
            !shown.contains("REPOTOKEN") && !shown.contains("SECRETSIG"),
            "{shown}"
        );
        assert!(
            index.to_json().contains("SECRETSIG"),
            "the original is untouched"
        );
    }

    #[test]
    fn binaries_are_read_and_validated() {
        let good = format!(
            r#"{{ "5.5": {{ "Win64": {{ "url": "https://github.com/AstreoX/uBotA/releases/download/v1/A-1.0.0-UE5.5-Win64.zip", "sha256": "{SHA}", "size": 4096 }} }},
                 "5.6": {{ "Win64": {{ "url": "file:///D:/mirror/A.zip", "sha256": "{SHA}" }} }},
                 "5.7": {{ "Win64": {{ "url": "D:\\mirror\\A.zip", "sha256": "{SHA}" }} }} }}"#
        );
        let text = with_binaries(&good);
        let validation = validate_text(&text);
        assert!(validation.errors.is_empty(), "{:#?}", validation.errors);
        assert!(validation.warnings.is_empty(), "{:#?}", validation.warnings);
        let index = parse(&text).unwrap();
        let version = index.find("A").unwrap().latest().unwrap();
        let binary = version.prebuilt("5.5", "Win64").unwrap();
        assert_eq!(binary.sha256, SHA);
        assert_eq!(binary.size, Some(4096));
        assert_eq!(
            version.prebuilt("5.6", "Win64").unwrap().url,
            "file:///D:/mirror/A.zip"
        );
        assert_eq!(version.prebuilt("5.7", "Win64").unwrap().size, None);
        assert!(version.prebuilt("5.4", "Win64").is_none());
        assert!(version.prebuilt("5.5", "Linux").is_none());
        // The merged index keeps the entries as written.
        let merged = MergedIndex::merge(&[SourceIndex {
            id: "t".into(),
            index: index.clone(),
        }]);
        let reparsed = parse(&merged.to_file().to_json()).unwrap();
        assert_eq!(
            reparsed.find("A").unwrap().versions[0].binaries,
            index.find("A").unwrap().versions[0].binaries
        );

        // null and absent are "no binaries".
        assert!(validate_text(&with_binaries("null")).errors.is_empty());
        let none = parse(&with_binaries("null")).unwrap();
        assert!(none.find("A").unwrap().versions[0]
            .prebuilt("5.5", "Win64")
            .is_none());

        let bad = format!(
            r#"{{ "5": {{ "Win64": {{ "url": "https://h/x.zip", "sha256": "{SHA}" }} }},
                 "5.5.1": {{}},
                 "five": {{}},
                 "5.4": {{ "Win64": {{ "url": "http://h/x.zip", "sha256": "{SHA}" }} }},
                 "5.3": {{ "Win64": {{ "url": "https://h/x.zip", "sha256": "{upper}" }} }},
                 "5.2": {{ "Win64": {{ "url": "https://h/x.zip", "sha256": "abc" }} }},
                 "5.1": {{ "Win64": {{ "sha256": "{SHA}" }} }},
                 "5.0": {{ "Win64": {{ "url": "git@h:x.zip", "sha256": "{SHA}", "size": 0 }} }},
                 "3.9": {{ "Win64": {{ "url": "https://evil.example.net\\@trusted.example.com/a.zip", "sha256": "{SHA}" }} }},
                 "4.27": {{ "Win64": {{ "url": "relative/x.zip", "sha256": "{SHA}", "size": 3000000000 }} }},
                 "4.26": "x",
                 "4.25": {{ "Win64": "x", "Linux": {{ "url": "https://h/l.zip", "sha256": "{SHA}" }} }},
                 "4.24": {{ "Win64": {{ "url": "https://user:SECRET@h/x.zip", "sha256": "{SHA}", "mirror": "y" }} }} }}"#,
            upper = SHA.to_uppercase()
        );
        let validation = validate_text(&with_binaries(&bad));
        let errors = validation.errors.join("\n");
        let warnings = validation.warnings.join("\n");
        for fragment in [
            "binaries[\"5\"]: '5' is not an engine version",
            "binaries[\"5.5.1\"]: '5.5.1' is not an engine version",
            "binaries[\"five\"]",
            "binaries[\"5.4\"][\"Win64\"]: url 'http://h/x.zip' is not allowed: plain http://",
            "binaries[\"5.3\"][\"Win64\"]: \"sha256\" must be 64 lower-case hex digits",
            "binaries[\"5.2\"][\"Win64\"]: \"sha256\" must be 64",
            "binaries[\"5.1\"][\"Win64\"]: \"url\" is missing",
            "binaries[\"5.0\"][\"Win64\"]: url 'git@h:x.zip' is not allowed",
            "\"size\" must be a positive number of bytes",
            "url 'relative/x.zip' is not allowed",
            "\"size\" 3000000000 is over the limit",
            "binaries[\"3.9\"][\"Win64\"]: url 'https://evil.example.net\\@trusted.example.com/a.zip' is not allowed: backslashes are not allowed",
            "binaries[\"4.26\"]: must be an object of platforms",
            "binaries[\"4.25\"][\"Win64\"]: must be an object",
        ] {
            assert!(
                errors.contains(fragment),
                "missing error '{fragment}' in:\n{errors}"
            );
        }
        for fragment in [
            "binaries[\"4.25\"][\"Linux\"]: unknown platform 'Linux' is ignored",
            "binaries[\"4.24\"][\"Win64\"]: unknown key \"mirror\" is ignored",
        ] {
            assert!(
                warnings.contains(fragment),
                "missing warning '{fragment}' in:\n{warnings}"
            );
        }
        assert!(!errors.contains("SECRET") && !warnings.contains("SECRET"));
        let validation = validate_text(&with_binaries("[]"));
        assert!(validation.errors[0].contains("binaries: must be an object"));

        // The reader stays tolerant: a broken entry does not make the index unreadable, and
        // only usable entries are offered.
        let index = parse(&with_binaries(&bad)).unwrap();
        let version = &index.find("A").unwrap().versions[0];
        for engine in [
            "5", "5.4", "5.3", "5.2", "5.1", "5.0", "4.27", "4.26", "4.25", "3.9",
        ] {
            assert!(version.prebuilt(engine, "Win64").is_none(), "{engine}");
        }
        assert!(
            version.prebuilt("4.24", "Win64").is_some(),
            "unknown keys are fine"
        );
        let redacted = index.redacted().to_json();
        assert!(!redacted.contains("SECRET"), "{redacted}");
        assert!(redacted.contains("https://***@h/x.zip"));

        // Planned versions offer no binaries.
        let planned = r#"{ "formatVersion": 2, "name": "t", "packages": [
            { "name": "A", "description": { "en": "a", "zh-CN": "a" }, "layer": "Content",
              "versions": [ { "version": "1.0.0", "planned": true,
                              "binaries": { "5.5": { "Win64": { "url": "https://h/x.zip", "sha256": "SHA" } } } } ] } ] }"#
            .replace("SHA", SHA);
        let validation = validate_text(&planned);
        assert!(validation.errors.is_empty(), "{:?}", validation.errors);
        assert!(validation
            .warnings
            .join("\n")
            .contains("planned versions ignore \"binaries\""));
        let index = parse(&planned).unwrap();
        assert!(index.find("A").unwrap().versions[0]
            .prebuilt("5.5", "Win64")
            .is_none());

        assert!(is_engine_key("5.5") && is_engine_key("10.12"));
        for key in ["5", "5.", ".5", "5.5.1", "v5.5", "5.x", ""] {
            assert!(!is_engine_key(key), "{key}");
        }
        assert!(is_sha256_hex(SHA));
        assert!(!is_sha256_hex(&SHA.to_uppercase()) && !is_sha256_hex(&SHA[1..]));
    }

    #[test]
    fn safe_refs() {
        // The vectors of UBotPackageTests.cpp (FUBotPackageIndex::IsSafeGitRef): both sides
        // must accept and refuse the same refs.
        for safe in [
            "v0.1.0",
            "release/1.0",
            "0123abc",
            "feature_x-2",
            "v1.0.0-rc.1+build",
        ] {
            assert!(is_safe_ref(safe), "{safe}");
        }
        for unsafe_ref in [
            "",
            "-x",
            "--upload-pack=calc",
            "a b",
            "a\tb",
            "a..b",
            "a~1",
            "a^",
            "a:b",
            "a?",
            "a*",
            "a[",
            "a\\b",
            "a\"b",
            "a\u{7f}b",
        ] {
            assert!(!is_safe_ref(unsafe_ref), "{unsafe_ref:?}");
        }

        let text = r#"{ "formatVersion": 2, "packages": [
            { "name": "A", "repository": "https://h/a.git", "versions": [ { "version": "1.0.0", "ref": "v1\"" } ] }
        ] }"#;
        let errors = validate_text(text).errors.join("\n");
        assert!(
            errors.contains("ref 'v1\"' is not a safe git ref"),
            "{errors}"
        );
    }

    #[test]
    fn merge_first_source_wins() {
        let first = parse(SAMPLE).unwrap();
        let mut second = parse(SAMPLE).unwrap();
        second.packages[0].name = "CORE".into();
        second.packages[0].tags = vec!["second".into()];
        second.packages.push(IndexPackage {
            name: "Extra".into(),
            ..Default::default()
        });
        second.recipes.push(IndexRecipe {
            id: "other".into(),
            ..Default::default()
        });
        second.replacements[0].replacement = "Core".into();
        second.replacements.push(Replacement {
            legacy: "Older".into(),
            replacement: "Core".into(),
            redirects: None,
        });

        let merged = MergedIndex::merge(&[
            SourceIndex {
                id: "one".into(),
                index: first,
            },
            SourceIndex {
                id: "two".into(),
                index: second,
            },
        ]);
        assert_eq!(merged.packages.len(), 4);
        let core = merged.find("core").unwrap();
        assert_eq!(core.source, "one");
        assert!(core.package.tags.is_empty());
        assert_eq!(merged.find("extra").unwrap().source, "two");
        assert_eq!(
            merged
                .recipes
                .iter()
                .map(|r| r.recipe.id.as_str())
                .collect::<Vec<_>>(),
            ["r", "other"]
        );
        assert_eq!(merged.replacements.len(), 2);
        assert_eq!(merged.replacements[0].replacement, "Sensor");
        assert!(merged.is_available("Sensor"));
        assert!(!merged.is_available("Future"));
        assert!(!merged.is_available("Nobody"));

        let file = merged.to_file();
        assert_eq!(file.name, "merged");
        let reparsed = parse(&file.to_json()).unwrap();
        assert_eq!(reparsed.packages.len(), 4);
        assert!(reparsed.find("Core").unwrap().versions[2].planned);
    }
}
