//! Prebuilt binaries (SPEC §1, §5): the install marker the manager writes into a package folder it
//! installed from an archive, the archive layout (extracting one safely, packing a built plugin),
//! and the index entries that point at archives.
//!
//! Archive layout: exactly one top-level folder named after the plugin (`UBotSensor/`) holding
//! `UBotSensor.uplugin`, `Binaries/Win64` (`UnrealEditor-*.dll`, `UnrealEditor.modules`), `Source`,
//! `Resources`, `Config` and so on; no `.git`, `Intermediate` or `Saved`, and no `*.pdb`, `*.exp`
//! or `*.lib` unless packed with symbols.

use std::fs;
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::index::{is_engine_key, to_pretty_json, PrebuiltBinary};
use crate::paths;
use crate::version::SemVer;

/// The marker the manager writes after installing an archive (never taken from the archive).
pub const MARKER_FILE: &str = ".ubot-install.json";
pub const MARKER_KIND: &str = "prebuilt";
/// The only platform prebuilt binaries are installed for.
pub const PLATFORM: &str = "Win64";
/// An archive extracts to at most this much; a zip bomb stops here.
pub const MAX_EXTRACTED_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 200_000;
const MAX_MARKER_BYTES: u64 = 64 * 1024;
const COPY_BUFFER: usize = 64 * 1024;

/// Why a long operation stopped early.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    Canceled,
    Failed(String),
}

impl Stop {
    pub fn failed(text: impl Into<String>) -> Stop {
        Stop::Failed(text.into())
    }
}

/// `<plugin dir>/.ubot-install.json`: what the manager installed there from an archive.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallMarker {
    /// Always "prebuilt".
    pub kind: String,
    pub package: String,
    pub version: String,
    /// `Major.Minor` of the engine the binaries were built for.
    pub engine: String,
    pub platform: String,
    /// Where the archive came from, credentials redacted.
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub sha256: String,
    /// RFC 3339.
    #[serde(default)]
    pub installed_at: String,
}

impl InstallMarker {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join(MARKER_FILE)
    }

    /// The marker in `dir` when it is a valid one for `package`.
    pub fn read(dir: &Path, package: &str) -> Option<InstallMarker> {
        let path = Self::path(dir);
        let metadata = fs::metadata(&path).ok()?;
        if !metadata.is_file() || metadata.len() > MAX_MARKER_BYTES {
            return None;
        }
        let (text, _) = paths::read_text(&path).ok()?;
        let marker: InstallMarker = serde_json::from_str(&text).ok()?;
        marker.is_valid_for(package).then_some(marker)
    }

    pub fn is_valid_for(&self, package: &str) -> bool {
        self.kind == MARKER_KIND
            && self.package.eq_ignore_ascii_case(package.trim())
            && SemVer::parse(&self.version).is_some()
            && is_engine_key(&self.engine)
            && !self.platform.trim().is_empty()
    }

    pub fn write(&self, dir: &Path) -> io::Result<()> {
        paths::write_atomic(&Self::path(dir), to_pretty_json(self).as_bytes())
    }
}

/// `Major.Minor` of an engine version ("5.5.4" → "5.5").
pub fn engine_major_minor(version: &str) -> Option<String> {
    let version = SemVer::parse(version)?;
    Some(format!("{}.{}", version.major, version.minor))
}

/// The file name of a package archive: `<Name>-<version>-UE<Major.Minor>-Win64.zip`.
pub fn archive_name(package: &str, version: &str, engine: &str) -> String {
    format!("{package}-{version}-UE{engine}-{PLATFORM}.zip")
}

/// `Binaries/Win64/UnrealEditor-*.dll` exists in a plugin folder: the editor can load it without
/// a build.
pub fn has_editor_binaries(dir: &Path) -> bool {
    fs::read_dir(dir.join("Binaries").join(PLATFORM)).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            name.starts_with("unrealeditor-")
                && name.ends_with(".dll")
                && entry.file_type().is_ok_and(|kind| kind.is_file())
        })
    })
}

/// Lower-case hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// SHA-256 of a file, lower-case hex.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; COPY_BUFFER];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Names that start like this, directly in a plugin folder, belong to the manager.
pub const RESERVED_PREFIX: &str = ".ubot-";

const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// The path components of an archive entry, or why the entry may not be extracted: absolute
/// paths, drive letters (and NTFS streams), `.` and `..`, and names Windows cannot hold.
fn entry_components(name: &str) -> Result<Vec<String>, String> {
    if name.contains('\0') || name.chars().any(char::is_control) {
        return Err(format!("{name:?} has control characters in its name"));
    }
    let normalized = name.replace('\\', "/");
    if normalized.starts_with('/') {
        return Err(format!("'{name}' is an absolute path"));
    }
    if normalized.contains(':') {
        return Err(format!("'{name}' names a drive or a stream"));
    }
    let parts: Vec<String> = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect();
    if parts.is_empty() {
        return Err("an entry has no name".into());
    }
    for part in &parts {
        if part == "." || part == ".." {
            return Err(format!("'{name}' leaves the folder it belongs to"));
        }
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if part.ends_with('.')
            || part.ends_with(' ')
            || RESERVED_NAMES.contains(&stem.trim_end())
            || part.contains(['<', '>', '"', '|', '?', '*'])
        {
            return Err(format!("'{name}' is not a valid Windows file name"));
        }
    }
    Ok(parts)
}

fn zip_error(error: zip::result::ZipError) -> Stop {
    Stop::Failed(format!("the archive cannot be read: {error}"))
}

/// Extracts a package archive into `into` (which should not exist yet) and returns the folder
/// of the plugin inside it (`<into>/<Package>`).
///
/// Every entry is checked before anything is written: no absolute paths, drive letters, `..` or
/// symbolic links, no `.git` folder (the install kind must not be decided by the archive), nothing
/// named `.ubot-*` directly in the plugin folder (the manager writes its own install marker), and
/// exactly one top-level folder, named after the package (any case). The extracted size is capped
/// at [`MAX_EXTRACTED_BYTES`]. `cancel` is checked between and inside
/// entries; the caller removes `into` after a failure or cancel.
pub fn extract_package(
    archive_path: &Path,
    package: &str,
    into: &Path,
    cancel: &AtomicBool,
) -> Result<PathBuf, Stop> {
    let file = fs::File::open(archive_path)
        .map_err(|error| Stop::Failed(format!("cannot open the archive: {error}")))?;
    let mut archive = zip::ZipArchive::new(io::BufReader::new(file))
        .map_err(|error| Stop::Failed(format!("the download is not a zip archive: {error}")))?;
    if archive.is_empty() {
        return Err(Stop::failed("the archive is empty"));
    }
    if archive.len() > MAX_ENTRIES {
        return Err(Stop::Failed(format!(
            "the archive has more than {MAX_ENTRIES} entries"
        )));
    }

    let mut top: Option<String> = None;
    let mut entries: Vec<(usize, Vec<String>, bool)> = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index).map_err(zip_error)?;
        let name = entry.name().map_err(zip_error)?.into_owned();
        if entry.is_symlink() {
            return Err(Stop::Failed(format!("'{name}' is a symbolic link")));
        }
        if entry.encrypted() {
            return Err(Stop::Failed(format!("'{name}' is encrypted")));
        }
        let parts = entry_components(&name).map_err(Stop::Failed)?;
        if parts.iter().any(|part| part.eq_ignore_ascii_case(".git")) {
            return Err(Stop::Failed(format!(
                "'{name}' is inside a .git folder; a package archive holds no git data"
            )));
        }
        // `parts[0]` is the plugin folder: what sits directly in it and starts with `.ubot-` is
        // the manager's (the install marker), never the archive's.
        if parts
            .get(1)
            .is_some_and(|part| part.to_ascii_lowercase().starts_with(RESERVED_PREFIX))
        {
            return Err(Stop::Failed(format!(
                "'{name}' uses a name reserved for the manager ({RESERVED_PREFIX}*)"
            )));
        }
        let is_dir = entry.is_dir();
        if parts.len() == 1 && !is_dir {
            return Err(Stop::Failed(format!(
                "'{name}' lies at the top level; the archive must hold one folder"
            )));
        }
        match &top {
            None => top = Some(parts[0].clone()),
            Some(first) if !first.eq_ignore_ascii_case(&parts[0]) => {
                return Err(Stop::Failed(format!(
                    "the archive has more than one top-level folder ('{first}' and '{}')",
                    parts[0]
                )))
            }
            Some(_) => {}
        }
        entries.push((index, parts, is_dir));
    }
    let top = top.expect("the archive has entries");
    if !top.eq_ignore_ascii_case(package.trim()) {
        return Err(Stop::Failed(format!(
            "the archive's top folder is '{top}', expected '{package}'"
        )));
    }

    fs::create_dir_all(into)
        .map_err(|error| Stop::Failed(format!("cannot create {}: {error}", into.display())))?;
    let mut written: u64 = 0;
    let mut buffer = vec![0u8; COPY_BUFFER];
    for (index, parts, is_dir) in entries {
        if cancel.load(Ordering::SeqCst) {
            return Err(Stop::Canceled);
        }
        let target = parts
            .iter()
            .fold(into.to_path_buf(), |path, part| path.join(part));
        let create_error =
            |error: io::Error| Stop::Failed(format!("cannot create {}: {error}", target.display()));
        if is_dir {
            fs::create_dir_all(&target).map_err(create_error)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(create_error)?;
        }
        let mut out = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(out) => out,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(Stop::Failed(format!(
                    "the archive holds '{}' twice",
                    parts.join("/")
                )))
            }
            Err(error) => return Err(create_error(error)),
        };
        let mut entry = archive.by_index(index).map_err(zip_error)?;
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(Stop::Canceled);
            }
            let read = entry.read(&mut buffer).map_err(|error| {
                Stop::Failed(format!("cannot extract '{}': {error}", parts.join("/")))
            })?;
            if read == 0 {
                break;
            }
            written += read as u64;
            if written > MAX_EXTRACTED_BYTES {
                return Err(Stop::Failed(format!(
                    "the archive extracts to more than {} GiB",
                    MAX_EXTRACTED_BYTES / 1024 / 1024 / 1024
                )));
            }
            out.write_all(&buffer[..read]).map_err(create_error)?;
        }
    }
    Ok(into.join(top))
}

/// The descriptor `<Package>.uplugin` directly in `dir` (any case).
pub fn find_descriptor(dir: &Path, package: &str) -> Option<PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("uplugin"))
                && path
                    .file_stem()
                    .is_some_and(|stem| stem.to_string_lossy().eq_ignore_ascii_case(package.trim()))
        })
}

/// Whether a path inside a plugin folder (relative, `/` separators) goes into a package archive:
/// not `.git`, not the plugin's `Intermediate`, `Saved` or `DerivedDataCache`, nothing named
/// `.ubot-*` directly in it (the install marker), and no `*.pdb`, `*.exp` or `*.lib` unless
/// `with_symbols`.
pub fn is_packaged(relative: &str, is_dir: bool, with_symbols: bool) -> bool {
    let parts: Vec<&str> = relative
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let Some(last) = parts.last() else {
        return false;
    };
    if parts.iter().any(|part| part.eq_ignore_ascii_case(".git")) {
        return false;
    }
    if parts
        .first()
        .is_some_and(|first| first.to_ascii_lowercase().starts_with(RESERVED_PREFIX))
    {
        return false;
    }
    if parts.len() == 1 {
        let skipped_dir = ["Intermediate", "Saved", "DerivedDataCache"]
            .iter()
            .any(|name| last.eq_ignore_ascii_case(name));
        if is_dir && skipped_dir {
            return false;
        }
    }
    if !is_dir && !with_symbols {
        let lower = last.to_ascii_lowercase();
        if [".pdb", ".exp", ".lib"]
            .iter()
            .any(|extension| lower.ends_with(extension))
        {
            return false;
        }
    }
    true
}

/// What [`pack_package`] put into an archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packed {
    /// Entry names, sorted (`UBotSensor/UBotSensor.uplugin`, ...).
    pub entries: Vec<String>,
    /// The archive's size in bytes.
    pub size: u64,
    pub sha256: String,
}

fn collect_files(
    dir: &Path,
    prefix: &str,
    with_symbols: bool,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), String> {
    let entries =
        fs::read_dir(dir).map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let kind = fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?
            .file_type();
        if kind.is_symlink() {
            if !is_packaged(&relative, false, true) {
                continue;
            }
            return Err(format!(
                "{} is a symbolic link; an archive cannot hold links",
                path.display()
            ));
        }
        if !is_packaged(&relative, kind.is_dir(), with_symbols) {
            continue;
        }
        if kind.is_dir() {
            collect_files(&path, &relative, with_symbols, files)?;
        } else {
            files.push((relative, path));
        }
    }
    Ok(())
}

/// Packs the plugin folder `plugin_dir` into `archive_path` with the package layout (one
/// top-level folder named `package`). The archive is written next to its destination first and
/// renamed into place.
pub fn pack_package(
    plugin_dir: &Path,
    package: &str,
    archive_path: &Path,
    with_symbols: bool,
) -> Result<Packed, String> {
    let mut files = Vec::new();
    collect_files(plugin_dir, "", with_symbols, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    if files.is_empty() {
        return Err(format!("{} holds nothing to pack", plugin_dir.display()));
    }

    let mut temp_name = archive_path.file_name().unwrap_or_default().to_os_string();
    temp_name.push(".part");
    let temp = archive_path.with_file_name(temp_name);
    let result = (|| -> Result<(), String> {
        let file = fs::File::create(&temp)
            .map_err(|error| format!("cannot create {}: {error}", temp.display()))?;
        let mut writer = zip::ZipWriter::new(BufWriter::new(file));
        for (relative, path) in &files {
            let size = fs::metadata(path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?
                .len();
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .large_file(size >= u64::from(u32::MAX));
            writer
                .start_file(format!("{package}/{relative}"), options)
                .map_err(|error| format!("cannot write the archive: {error}"))?;
            let mut source = fs::File::open(path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            io::copy(&mut source, &mut writer)
                .map_err(|error| format!("cannot pack {}: {error}", path.display()))?;
        }
        let mut inner = writer
            .finish()
            .map_err(|error| format!("cannot write the archive: {error}"))?;
        inner
            .flush()
            .map_err(|error| format!("cannot write the archive: {error}"))?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    if archive_path.exists() {
        fs::remove_file(archive_path)
            .map_err(|error| format!("cannot replace {}: {error}", archive_path.display()))?;
    }
    fs::rename(&temp, archive_path)
        .map_err(|error| format!("cannot write {}: {error}", archive_path.display()))?;
    let size = fs::metadata(archive_path)
        .map_err(|error| format!("cannot read {}: {error}", archive_path.display()))?
        .len();
    let sha256 = sha256_file(archive_path)
        .map_err(|error| format!("cannot read {}: {error}", archive_path.display()))?;
    Ok(Packed {
        entries: files
            .into_iter()
            .map(|(relative, _)| format!("{package}/{relative}"))
            .collect(),
        size,
        sha256,
    })
}

/// The placeholders of a download URL template.
pub const URL_TEMPLATE_KEYS: &[&str] = &["repo", "ref", "file", "name", "version", "engine"];

/// The default download URL of a package archive: its GitHub release asset.
pub const DEFAULT_URL_TEMPLATE: &str =
    "https://github.com/AstreoX/{repo}/releases/download/{ref}/{file}";

/// Fills `{key}` placeholders. An unknown placeholder or an unclosed brace is an error.
pub fn expand_url_template(template: &str, values: &[(&str, &str)]) -> Result<String, String> {
    let mut result = String::with_capacity(template.len() + 64);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        result.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            return Err(format!(
                "the URL template '{template}' has an unclosed '{{'"
            ));
        };
        let key = &after[..close];
        match values.iter().find(|(name, _)| *name == key) {
            Some((_, value)) => result.push_str(value),
            None => {
                return Err(format!(
                    "the URL template uses {{{key}}}; known placeholders: {}",
                    URL_TEMPLATE_KEYS
                        .iter()
                        .map(|key| format!("{{{key}}}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                ))
            }
        }
        rest = &after[close + 1..];
    }
    result.push_str(rest);
    Ok(result)
}

/// One `binaries` entry to write into an index file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryUpdate {
    pub package: String,
    pub version: String,
    pub engine: String,
    pub platform: String,
    pub binary: PrebuiltBinary,
}

/// A JSON value with the byte range it occupies in the text.
#[derive(Debug)]
enum Node {
    Object { members: Vec<Member> },
    Array { items: Vec<(usize, usize, Node)> },
    Scalar,
}

#[derive(Debug)]
struct Member {
    key: String,
    key_start: usize,
    value_start: usize,
    value_end: usize,
    value: Node,
}

/// A minimal JSON scanner over text serde has already accepted: it records where values are, so
/// an entry can be changed without reformatting the rest of the file.
struct Scanner<'a> {
    text: &'a [u8],
    at: usize,
}

impl Scanner<'_> {
    fn skip_whitespace(&mut self) {
        while self
            .text
            .get(self.at)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
        {
            self.at += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Option<()> {
        self.skip_whitespace();
        (self.text.get(self.at) == Some(&byte)).then(|| self.at += 1)
    }

    fn string(&mut self) -> Option<()> {
        self.expect(b'"')?;
        loop {
            match self.text.get(self.at)? {
                b'\\' => self.at += 2,
                b'"' => {
                    self.at += 1;
                    return Some(());
                }
                _ => self.at += 1,
            }
        }
    }

    /// The next value and its byte range.
    fn value(&mut self) -> Option<(usize, usize, Node)> {
        self.skip_whitespace();
        let start = self.at;
        let node = match self.text.get(self.at)? {
            b'{' => {
                self.at += 1;
                let mut members = Vec::new();
                self.skip_whitespace();
                if self.text.get(self.at) == Some(&b'}') {
                    self.at += 1;
                } else {
                    loop {
                        self.skip_whitespace();
                        let key_start = self.at;
                        self.string()?;
                        let key: String =
                            serde_json::from_slice(&self.text[key_start..self.at]).ok()?;
                        self.expect(b':')?;
                        let (value_start, value_end, value) = self.value()?;
                        members.push(Member {
                            key,
                            key_start,
                            value_start,
                            value_end,
                            value,
                        });
                        self.skip_whitespace();
                        match self.text.get(self.at)? {
                            b',' => self.at += 1,
                            b'}' => {
                                self.at += 1;
                                break;
                            }
                            _ => return None,
                        }
                    }
                }
                Node::Object { members }
            }
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                self.skip_whitespace();
                if self.text.get(self.at) == Some(&b']') {
                    self.at += 1;
                } else {
                    loop {
                        items.push(self.value()?);
                        self.skip_whitespace();
                        match self.text.get(self.at)? {
                            b',' => self.at += 1,
                            b']' => {
                                self.at += 1;
                                break;
                            }
                            _ => return None,
                        }
                    }
                }
                Node::Array { items }
            }
            b'"' => {
                self.string()?;
                Node::Scalar
            }
            _ => {
                while self.text.get(self.at).is_some_and(|byte| {
                    !matches!(byte, b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r')
                }) {
                    self.at += 1;
                }
                if self.at == start {
                    return None;
                }
                Node::Scalar
            }
        };
        Some((start, self.at, node))
    }
}

fn member<'a>(node: &'a Node, key: &str) -> Option<&'a Member> {
    match node {
        Node::Object { members } => members.iter().find(|member| member.key == key),
        _ => None,
    }
}

fn item(node: &Node, position: usize) -> Option<&(usize, usize, Node)> {
    match node {
        Node::Array { items } => items.get(position),
        _ => None,
    }
}

/// The whitespace before `at` on its line, when nothing else precedes it there.
fn line_indent(text: &str, at: usize) -> Option<&str> {
    let line_start = text[..at].rfind('\n').map_or(0, |position| position + 1);
    let indent = &text[line_start..at];
    indent
        .chars()
        .all(|c| c == ' ' || c == '\t')
        .then_some(indent)
}

/// `value` pretty-printed with 4-space steps, continuation lines indented by `indent`.
fn render_at(value: &Value, indent: &str) -> String {
    let text = to_pretty_json(value);
    text.trim_end()
        .lines()
        .enumerate()
        .map(|(position, line)| {
            if position == 0 {
                line.to_string()
            } else {
                format!("{indent}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Sets `binaries` of `packages[package].versions[version]` in the text (LF line ends) without
/// touching anything else; `None` when the text does not have the expected shape.
fn edit_binaries(text: &str, package: usize, version: usize, binaries: &Value) -> Option<String> {
    let mut scanner = Scanner {
        text: text.as_bytes(),
        at: 0,
    };
    let (_, _, root) = scanner.value()?;
    let packages = member(&root, "packages")?;
    let (_, _, entry) = item(&packages.value, package)?;
    let versions = member(entry, "versions")?;
    let (object_start, _, version_node) = item(&versions.value, version)?;
    let Node::Object { members } = version_node else {
        return None;
    };
    let mut result = String::with_capacity(text.len() + 512);
    if let Some(existing) = members.iter().find(|member| member.key == "binaries") {
        let rendered = match line_indent(text, existing.key_start) {
            Some(indent) => render_at(binaries, indent),
            None => serde_json::to_string(binaries).ok()?,
        };
        result.push_str(&text[..existing.value_start]);
        result.push_str(&rendered);
        result.push_str(&text[existing.value_end..]);
    } else {
        let last = members.last()?;
        // An object written on one line gets the member on that line, too.
        let inline = text[*object_start..last.key_start].find('\n').is_none();
        let addition = match line_indent(text, last.key_start) {
            Some(indent) if !inline => {
                format!(",\n{indent}\"binaries\": {}", render_at(binaries, indent))
            }
            _ => format!(", \"binaries\": {}", serde_json::to_string(binaries).ok()?),
        };
        result.push_str(&text[..last.value_end]);
        result.push_str(&addition);
        result.push_str(&text[last.value_end..]);
    }
    Some(result)
}

/// Writes `binaries` entries into the text of an index file: each update sets
/// `binaries[engine][platform]` of the version it names, keeping the other engines and
/// platforms. The rest of the file keeps its key order and formatting (new text uses 4-space
/// indentation); a byte order mark and CRLF line ends are kept. When the file's layout cannot be
/// followed, the whole file is written with 4-space indentation instead.
pub fn update_index_binaries(text: &str, updates: &[BinaryUpdate]) -> Result<String, String> {
    let bom = text.starts_with('\u{feff}');
    let body = text.trim_start_matches('\u{feff}');
    let crlf = body.contains("\r\n");
    let mut value: Value =
        serde_json::from_str(body).map_err(|error| format!("not valid JSON: {error}"))?;
    let mut current = Some(body.replace("\r\n", "\n"));

    for update in updates {
        let wanted = SemVer::parse(&update.version)
            .ok_or_else(|| format!("'{}' is not a version", update.version))?;
        let packages = value
            .get_mut("packages")
            .and_then(Value::as_array_mut)
            .ok_or("the index has no \"packages\" array")?;
        let package_position = packages
            .iter()
            .position(|package| {
                package
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| name.eq_ignore_ascii_case(update.package.trim()))
            })
            .ok_or_else(|| format!("there is no package {}", update.package))?;
        let versions = packages[package_position]
            .get_mut("versions")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| format!("{} has no \"versions\" array", update.package))?;
        let version_position = versions
            .iter()
            .position(|version| {
                version.get("planned") != Some(&Value::Bool(true))
                    && version
                        .get("version")
                        .and_then(Value::as_str)
                        .and_then(SemVer::parse)
                        .is_some_and(|version| version == wanted)
            })
            .ok_or_else(|| format!("there is no {} {}", update.package, update.version))?;
        let version = versions[version_position]
            .as_object_mut()
            .ok_or_else(|| format!("{} {} is not an object", update.package, update.version))?;

        let mut engines: Map<String, Value> = version
            .get("binaries")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut platforms: Map<String, Value> = engines
            .get(&update.engine)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        platforms.insert(
            update.platform.clone(),
            serde_json::to_value(&update.binary).map_err(|error| error.to_string())?,
        );
        engines.insert(update.engine.clone(), Value::Object(platforms));
        let binaries = Value::Object(engines);
        version.insert("binaries".into(), binaries.clone());

        current = current
            .and_then(|text| edit_binaries(&text, package_position, version_position, &binaries));
    }

    // The edited text must say exactly what the edited value says; otherwise write the value.
    let mut out = match current {
        Some(text) if serde_json::from_str::<Value>(&text).ok().as_ref() == Some(&value) => text,
        _ => to_pretty_json(&value),
    };
    if crlf {
        out = out.replace('\n', "\r\n");
    }
    if bom {
        out.insert(0, '\u{feff}');
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Cursor;

    fn binary(n: u8) -> PrebuiltBinary {
        PrebuiltBinary {
            url: format!("https://h/{n}.zip"),
            sha256: format!("{n:x}").repeat(64)[..64].to_string(),
            size: Some(u64::from(n) * 100),
        }
    }

    /// A zip with the given entries: `name` → contents (`None` for a directory).
    pub fn zip_bytes(entries: &[(&str, Option<&[u8]>)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, contents) in entries {
            match contents {
                Some(contents) => {
                    writer.start_file(*name, options).unwrap();
                    writer.write_all(contents).unwrap();
                }
                None => writer.add_directory(*name, options).unwrap(),
            }
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn markers_are_validated() {
        let dir = tempfile::tempdir().unwrap();
        assert!(InstallMarker::read(dir.path(), "UBotSensor").is_none());
        let marker = InstallMarker {
            kind: MARKER_KIND.into(),
            package: "UBotSensor".into(),
            version: "0.1.0".into(),
            engine: "5.5".into(),
            platform: PLATFORM.into(),
            url: "https://***@h/x.zip".into(),
            sha256: "a".repeat(64),
            installed_at: "2026-10-10T08:00:00Z".into(),
        };
        marker.write(dir.path()).unwrap();
        let text = fs::read_to_string(dir.path().join(MARKER_FILE)).unwrap();
        assert!(text.starts_with("{\n    \"kind\": \"prebuilt\",\n    \"package\": \"UBotSensor\""));
        assert!(text.contains("\"installedAt\": \"2026-10-10T08:00:00Z\""));
        assert_eq!(
            InstallMarker::read(dir.path(), "ubotsensor"),
            Some(marker.clone())
        );
        assert!(
            InstallMarker::read(dir.path(), "UBotCore").is_none(),
            "a marker of another package"
        );
        for broken in [
            InstallMarker {
                kind: "git".into(),
                ..marker.clone()
            },
            InstallMarker {
                engine: "5".into(),
                ..marker.clone()
            },
            InstallMarker {
                version: "x".into(),
                ..marker.clone()
            },
        ] {
            broken.write(dir.path()).unwrap();
            assert!(
                InstallMarker::read(dir.path(), "UBotSensor").is_none(),
                "{broken:?}"
            );
        }
        fs::write(dir.path().join(MARKER_FILE), "{ not json").unwrap();
        assert!(InstallMarker::read(dir.path(), "UBotSensor").is_none());
    }

    #[test]
    fn names_and_helpers() {
        assert_eq!(engine_major_minor("5.5.4").as_deref(), Some("5.5"));
        assert_eq!(engine_major_minor("5.10").as_deref(), Some("5.10"));
        assert_eq!(engine_major_minor("nope"), None);
        assert_eq!(
            archive_name("UBotSensor", "0.1.0", "5.5"),
            "UBotSensor-0.1.0-UE5.5-Win64.zip"
        );
        assert_eq!(hex(&[0, 15, 255]), "000fff");

        let dir = tempfile::tempdir().unwrap();
        assert!(!has_editor_binaries(dir.path()));
        let win64 = dir.path().join("Binaries").join("Win64");
        fs::create_dir_all(&win64).unwrap();
        fs::write(win64.join("UnrealEditor.modules"), "{}").unwrap();
        assert!(!has_editor_binaries(dir.path()));
        fs::write(win64.join("UnrealEditor-UBotSensor.dll"), "x").unwrap();
        assert!(has_editor_binaries(dir.path()));
    }

    #[test]
    fn url_templates() {
        let values = [("repo", "uBotSensor"), ("ref", "v0.1.0"), ("file", "X.zip")];
        assert_eq!(
            expand_url_template(DEFAULT_URL_TEMPLATE, &values).unwrap(),
            "https://github.com/AstreoX/uBotSensor/releases/download/v0.1.0/X.zip"
        );
        assert!(expand_url_template("https://h/{nope}", &values)
            .unwrap_err()
            .contains("{nope}"));
        assert!(expand_url_template("https://h/{file", &values)
            .unwrap_err()
            .contains("unclosed"));
        assert_eq!(expand_url_template("plain", &values).unwrap(), "plain");
    }

    #[test]
    fn packaging_rules() {
        for (path, is_dir, symbols, packed) in [
            ("UBotSensor.uplugin", false, false, true),
            ("Binaries", true, false, true),
            (
                "Binaries/Win64/UnrealEditor-UBotSensor.dll",
                false,
                false,
                true,
            ),
            (
                "Binaries/Win64/UnrealEditor-UBotSensor.pdb",
                false,
                false,
                false,
            ),
            (
                "Binaries/Win64/UnrealEditor-UBotSensor.PDB",
                false,
                true,
                true,
            ),
            ("Binaries/Win64/x.exp", false, false, false),
            ("Binaries/Win64/x.lib", false, false, false),
            ("Intermediate", true, false, false),
            ("Saved", true, true, false),
            ("DerivedDataCache", true, false, false),
            ("Source/Intermediate", true, false, true),
            (".git", true, false, false),
            (".git", false, false, false),
            ("Source/.git/config", false, false, false),
            (".gitignore", false, false, true),
            (MARKER_FILE, false, false, false),
            (".ubot-extract-X", true, false, false),
            (".ubot-extract-X/a.txt", false, false, false),
            ("Source/.ubot-notes.txt", false, false, true),
            ("Resources/Icon128.png", false, false, true),
        ] {
            assert_eq!(is_packaged(path, is_dir, symbols), packed, "{path}");
        }
    }

    fn extract(bytes: &[u8], package: &str) -> (tempfile::TempDir, Result<PathBuf, Stop>) {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        fs::write(&archive, bytes).unwrap();
        let result = extract_package(
            &archive,
            package,
            &dir.path().join("out"),
            &AtomicBool::new(false),
        );
        (dir, result)
    }

    #[test]
    fn extracts_the_package_folder() {
        let bytes = zip_bytes(&[
            ("UBotSensor/", None),
            ("UBotSensor/UBotSensor.uplugin", Some(b"{}")),
            (
                "UBotSensor/Binaries/Win64/UnrealEditor-UBotSensor.dll",
                Some(b"dll"),
            ),
            (
                "UBotSensor\\Config\\FilterPlugin.ini",
                Some(b"[FilterPlugin]"),
            ),
        ]);
        let (dir, result) = extract(&bytes, "ubotsensor");
        let top = result.unwrap();
        assert_eq!(top, dir.path().join("out").join("UBotSensor"));
        assert_eq!(fs::read(top.join("UBotSensor.uplugin")).unwrap(), b"{}");
        assert!(has_editor_binaries(&top));
        assert!(top.join("Config").join("FilterPlugin.ini").is_file());
        assert_eq!(
            find_descriptor(&top, "UBOTSENSOR"),
            Some(top.join("UBotSensor.uplugin"))
        );
    }

    #[test]
    fn refuses_archives_that_leave_their_folder_or_hold_more() {
        let cases: Vec<(Vec<u8>, &str)> = vec![
            (
                zip_bytes(&[("UBotSensor/../../evil.txt", Some(b"x"))]),
                "leaves the folder",
            ),
            (zip_bytes(&[("/abs/evil.txt", Some(b"x"))]), "absolute path"),
            (
                zip_bytes(&[("\\abs\\evil.txt", Some(b"x"))]),
                "absolute path",
            ),
            (zip_bytes(&[("C:/evil.txt", Some(b"x"))]), "drive"),
            (
                zip_bytes(&[("UBotSensor/x.txt:stream", Some(b"x"))]),
                "drive or a stream",
            ),
            (
                zip_bytes(&[
                    ("UBotSensor/UBotSensor.uplugin", Some(b"{}")),
                    ("Other/x.txt", Some(b"x")),
                ]),
                "more than one top-level folder",
            ),
            (
                zip_bytes(&[
                    ("UBotSensor/UBotSensor.uplugin", Some(b"{}")),
                    ("readme.txt", Some(b"x")),
                ]),
                "top level",
            ),
            (
                zip_bytes(&[("UBotCore/UBotCore.uplugin", Some(b"{}"))]),
                "expected 'UBotSensor'",
            ),
            (
                zip_bytes(&[("UBotSensor/CON", Some(b"x"))]),
                "Windows file name",
            ),
            (zip_bytes(&[]), "empty"),
            (b"not a zip".to_vec(), "not a zip archive"),
        ];
        for (bytes, fragment) in cases {
            let (dir, result) = extract(&bytes, "UBotSensor");
            match result {
                Err(Stop::Failed(error)) => {
                    assert!(error.contains(fragment), "{fragment}: {error}")
                }
                other => panic!("{fragment}: {other:?}"),
            }
            assert!(
                !dir.path().join("evil.txt").exists()
                    && !dir.path().parent().unwrap().join("evil.txt").exists(),
                "nothing escaped"
            );
            assert!(
                !dir.path().join("out").exists(),
                "{fragment}: nothing is written before every entry is checked"
            );
        }

        // A symbolic link entry.
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer
            .start_file("UBotSensor/UBotSensor.uplugin", options)
            .unwrap();
        writer.write_all(b"{}").unwrap();
        writer
            .add_symlink("UBotSensor/link", "../../outside", options)
            .unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let (_dir, result) = extract(&bytes, "UBotSensor");
        assert!(
            matches!(&result, Err(Stop::Failed(error)) if error.contains("symbolic link")),
            "{result:?}"
        );
    }

    #[test]
    fn refuses_git_data_and_names_the_manager_owns() {
        let cases: Vec<(Vec<u8>, &str)> = vec![
            (
                zip_bytes(&[
                    ("UBotSensor/UBotSensor.uplugin", Some(b"{}")),
                    ("UBotSensor/.git/HEAD", Some(b"ref: refs/heads/main")),
                ]),
                ".git folder",
            ),
            (
                zip_bytes(&[
                    ("UBotSensor/UBotSensor.uplugin", Some(b"{}")),
                    ("UBotSensor/.GIT/config", Some(b"[core]")),
                ]),
                ".git folder",
            ),
            (
                zip_bytes(&[
                    ("UBotSensor/UBotSensor.uplugin", Some(b"{}")),
                    ("UBotSensor/Source/x/.git", Some(b"gitdir: ../../elsewhere")),
                ]),
                ".git folder",
            ),
            (
                zip_bytes(&[
                    ("UBotSensor/UBotSensor.uplugin", Some(b"{}")),
                    ("UBotSensor/.git/", None),
                ]),
                ".git folder",
            ),
            (
                zip_bytes(&[
                    ("UBotSensor/UBotSensor.uplugin", Some(b"{}")),
                    (
                        "UBotSensor/.ubot-install.json",
                        Some(b"{\"kind\":\"prebuilt\"}"),
                    ),
                ]),
                "reserved for the manager",
            ),
            (
                zip_bytes(&[
                    ("UBotSensor/UBotSensor.uplugin", Some(b"{}")),
                    ("UBotSensor/.UBOT-extract/x.txt", Some(b"x")),
                ]),
                "reserved for the manager",
            ),
        ];
        for (bytes, fragment) in cases {
            let (dir, result) = extract(&bytes, "UBotSensor");
            match result {
                Err(Stop::Failed(error)) => {
                    assert!(error.contains(fragment), "{fragment}: {error}")
                }
                other => panic!("{fragment}: {other:?}"),
            }
            assert!(
                !dir.path().join("out").exists(),
                "{fragment}: nothing is written before every entry is checked"
            );
        }

        // Names that merely look similar are packages' own.
        let bytes = zip_bytes(&[
            ("UBotSensor/UBotSensor.uplugin", Some(b"{}")),
            ("UBotSensor/.gitignore", Some(b"*.log")),
            ("UBotSensor/Source/.ubot-notes.txt", Some(b"x")),
        ]);
        let (_dir, result) = extract(&bytes, "UBotSensor");
        let top = result.unwrap();
        assert!(top.join(".gitignore").is_file());
    }

    #[test]
    fn extraction_can_be_canceled() {
        let bytes = zip_bytes(&[("UBotSensor/UBotSensor.uplugin", Some(b"{}"))]);
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        fs::write(&archive, bytes).unwrap();
        let result = extract_package(
            &archive,
            "UBotSensor",
            &dir.path().join("out"),
            &AtomicBool::new(true),
        );
        assert_eq!(result, Err(Stop::Canceled));
    }

    #[test]
    fn packs_with_the_package_layout() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join("uBotSensor");
        for (path, text) in [
            ("UBotSensor.uplugin", "{}"),
            ("Binaries/Win64/UnrealEditor-UBotSensor.dll", "dll"),
            ("Binaries/Win64/UnrealEditor-UBotSensor.pdb", "pdb"),
            ("Binaries/Win64/UnrealEditor.modules", "{}"),
            ("Intermediate/Build/x.obj", "obj"),
            ("Saved/Logs/x.log", "log"),
            (".git/HEAD", "ref"),
            ("Source/UBotSensor/UBotSensor.Build.cs", "cs"),
            (MARKER_FILE, "{}"),
        ] {
            let path = plugin.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let archive = dir.path().join("out.zip");
        let packed = pack_package(&plugin, "UBotSensor", &archive, false).unwrap();
        assert_eq!(
            packed.entries,
            [
                "UBotSensor/Binaries/Win64/UnrealEditor-UBotSensor.dll",
                "UBotSensor/Binaries/Win64/UnrealEditor.modules",
                "UBotSensor/Source/UBotSensor/UBotSensor.Build.cs",
                "UBotSensor/UBotSensor.uplugin",
            ]
        );
        assert_eq!(packed.size, fs::metadata(&archive).unwrap().len());
        assert_eq!(packed.sha256, sha256_file(&archive).unwrap());
        assert!(!dir.path().join("out.zip.part").exists());

        // It extracts as a package.
        let top = extract_package(
            &archive,
            "UBotSensor",
            &dir.path().join("x"),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(has_editor_binaries(&top));

        let packed = pack_package(&plugin, "UBotSensor", &archive, true).unwrap();
        assert!(packed
            .entries
            .contains(&"UBotSensor/Binaries/Win64/UnrealEditor-UBotSensor.pdb".to_string()));
    }

    const INDEX: &str = "{\n    \"$schema\": \"./index.schema.json\",\n    \"formatVersion\": 2,\n    \"packages\": [\n        {\n            \"name\": \"UBotCore\",\n            \"tags\": [ \"core\" ],\n            \"versions\": [\n                {\n                    \"version\": \"0.1.0\",\n                    \"ref\": \"v0.1.0\",\n                    \"requires\": [],\n                    \"provides\": [ \"Core.Clock\" ]\n                }\n            ]\n        },\n        {\n            \"name\": \"UBotSensor\",\n            \"versions\": [ { \"version\": \"0.1.0\", \"ref\": \"v0.1.0\" }, { \"version\": \"0.2.0\", \"planned\": true } ]\n        }\n    ]\n}\n";

    fn update(package: &str, version: &str, engine: &str, n: u8) -> BinaryUpdate {
        BinaryUpdate {
            package: package.into(),
            version: version.into(),
            engine: engine.into(),
            platform: PLATFORM.into(),
            binary: binary(n),
        }
    }

    #[test]
    fn index_updates_keep_the_rest_of_the_file() {
        let updated = update_index_binaries(
            INDEX,
            &[
                update("ubotcore", "0.1", "5.5", 1),
                update("UBotSensor", "0.1.0", "5.5", 2),
            ],
        )
        .unwrap();
        let expected_core = "                    \"provides\": [ \"Core.Clock\" ],\n                    \"binaries\": {\n                        \"5.5\": {\n                            \"Win64\": {\n                                \"url\": \"https://h/1.zip\",\n                                \"sha256\": \"1111111111111111111111111111111111111111111111111111111111111111\",\n                                \"size\": 100\n                            }\n                        }\n                    }\n                }\n";
        assert!(updated.contains(expected_core), "{updated}");
        assert!(
            updated.contains("\"tags\": [ \"core\" ],"),
            "untouched: {updated}"
        );
        assert!(
            updated.contains("{ \"version\": \"0.1.0\", \"ref\": \"v0.1.0\", \"binaries\": {\"5.5\":{\"Win64\":{\"url\":\"https://h/2.zip\""),
            "an inline object stays inline: {updated}"
        );
        assert!(updated.starts_with("{\n    \"$schema\""));
        let value: Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(
            value["packages"][0]["versions"][0]["binaries"]["5.5"]["Win64"]["size"],
            100
        );

        // A second engine is added next to the first; the same key is replaced.
        let again = update_index_binaries(
            &updated,
            &[
                update("UBotCore", "0.1.0", "5.6", 3),
                update("UBotCore", "0.1.0", "5.5", 4),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&again).unwrap();
        let binaries = &value["packages"][0]["versions"][0]["binaries"];
        assert_eq!(
            binaries.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["5.5", "5.6"]
        );
        assert_eq!(binaries["5.5"]["Win64"]["url"], "https://h/4.zip");
        assert_eq!(binaries["5.6"]["Win64"]["url"], "https://h/3.zip");
        assert!(again.contains("\"tags\": [ \"core\" ],"));

        // CRLF and a byte order mark are kept.
        let windows = format!("\u{feff}{}", INDEX.replace('\n', "\r\n"));
        let updated =
            update_index_binaries(&windows, &[update("UBotCore", "0.1.0", "5.5", 1)]).unwrap();
        assert!(updated.starts_with('\u{feff}'));
        assert!(
            !updated.replace("\r\n", "").contains('\n'),
            "only CRLF line ends"
        );
        assert!(updated.contains(&expected_core.replace('\n', "\r\n")));

        // Unknown packages and versions are errors; planned versions are not a target.
        for (package, version, fragment) in [
            ("Nope", "0.1.0", "no package Nope"),
            ("UBotCore", "9.0.0", "no UBotCore 9.0.0"),
            ("UBotSensor", "0.2.0", "no UBotSensor 0.2.0"),
        ] {
            let error =
                update_index_binaries(INDEX, &[update(package, version, "5.5", 1)]).unwrap_err();
            assert!(error.contains(fragment), "{error}");
        }
    }
}
