# uBot Manager — implementation spec

This is the contract every part of uBot Manager is built against: the package index (in the uBotCore
repo), the Rust core library and CLI, the Tauri desktop app, its web frontend, and the uBotCore /
uBotROS editor and runtime changes. When code and this document disagree, fix one of them; do not
let them drift.

## 0. Repositories and layout

| What | Where |
|---|---|
| uBot Manager (this repository, `AstreoX/uBotManager`) | `crates/ubot-core` (library), `crates/ubot-cli` (`ubot`), `src-tauri` (desktop shell), `app` (web frontend), `scripts` |
| uBot packages | `AstreoX/uBotCore`, `AstreoX/uBotSensor`, `AstreoX/uBotROS` (UE 5.5 plugins) |
| Package index (format 2) | `uBotCore/Index/index.json` (+ `index.schema.json`, `README.md`) |
| Visual design | the uBot Manager concept canvas: English and Simplified Chinese pages for every view and dialog, the UE panel, and a terminology and typography sheet; the frontend and the UE panel follow it |

Development layout assumed by the tests and `scripts/sync-index.mjs`: the package repositories live in
`../../Plugins/uBot/{uBotCore,uBotSensor,uBotROS}` relative to this repository's root (override with
`UBOTCORE_DIR`, `UBOT_PLUGINS_DIR` or `UBOTCORE_INDEX`). A content-only host project that loads them
through `AdditionalPluginDirectories` is the usual way to build and test the UE side.

Toolchain: stable Rust (MSVC on Windows), Node 20+ with npm, git, and for the UE side Unreal Engine 5.5
with Visual Studio 2022. The Tauri CLI comes from the `@tauri-apps/cli` npm package. Never edit or commit
`Binaries/`, `Intermediate/`, `Saved/`, `DerivedDataCache/`, `node_modules/` or `target/`, and close the
Unreal Editor before rebuilding C++.

## 1. Package index, format 2

Lives in the uBotCore repo at `Index/index.json`. Remote URL (default source):
`https://raw.githubusercontent.com/AstreoX/uBotCore/main/Index/index.json`.
JSON keys are camelCase (descriptor `.uplugin` keys stay PascalCase as UE requires).

```jsonc
{
  "$schema": "./index.schema.json",      // optional, ignored
  "formatVersion": 2,                     // required; a reader rejects any other value
  "name": "uBot",                         // display name of this index; informational, the UI does not show it
                                          // (the package list group is the fixed title "uBot Index" / "uBot 索引")
  "packages": [ Package ],
  "recipes": [ Recipe ],                  // optional
  "replacements": [ Replacement ]         // optional
}
Package = {
  "name": "UBotSensor",                   // required; UE plugin name; unique case-insensitively
  "friendlyName": "uBot Sensor",
  "description": { "en": "...", "zh-CN": "..." },   // "en" required, other locales optional
  "layer": "Foundation" | "Capability" | "Adapter" | "Content",
  "repository": "https://github.com/AstreoX/uBotSensor.git",  // git URL; required unless every version is planned
  "folder": "uBotSensor",                 // optional install folder; default = last segment of repository without ".git"
  "docsUrl": "https://...",               // optional
  "tags": ["sensor"],
  "versions": [ Version ]                 // any order; readers sort by semver
}
Version = {
  "version": "0.1.0",                     // required, SemVer
  "ref": "v0.1.0",                        // git tag (or branch/commit) to check out; required unless planned
  "planned": false,                       // true: announced, not installable; no ref
  "engine": "~5.5",                       // optional constraint over the UE version (e.g. 5.5.4)
  "requires": [ { "name": "UBotCore", "version": "^0.1.0", "optional": false } ],
  "enginePlugins": ["ChaosVehiclesPlugin"], // engine plugins that must be enabled
  "provides": ["Sensor.IMU"],             // capability ids
  "binaries": null                        // RESERVED for prebuilt binaries ({ "Win64": { "url", "sha256" } }); ignored today
}
Recipe = { "id": "ros-sensors", "name": {en, zh-CN}, "description": {en, zh-CN},
           "packages": ["UBotCore", ...], "checks": ["rosBridge"] }   // checks: known ids only, unknown ignored
Replacement = { "legacy": "AgentSensorCore", "replacement": "UBotSensor",
                "redirects": "Config/DefaultUBotSensor.ini" }          // redirects: path inside the replacement plugin, optional
```

Derived values:

- *latest version* = highest non-planned version (SemVer precedence). A package with no non-planned
  version is **planned**.
- A recipe is *available* when every package in it is available (not planned, present in the index).
- A reader (`index::parse`) is strict about the shape: the text is a JSON object (a leading BOM is
  dropped) with `formatVersion` 2 (a file with only the format 1 key `FormatVersion` is told so) and a
  `packages` array whose entries deserialize; `name`, when present, is a string. Unknown keys are
  ignored when reading and reported by validation.
- Validation (`ubot index validate <file>`, CI-style) is stricter than the shape above. **Errors**
  (exit 1):
  - the file does not parse as above;
  - package: `name` empty, not a UE plugin name (`[A-Za-z_][A-Za-z0-9_]*`) or used twice
    (case-insensitive); `layer` present but not Foundation/Capability/Adapter/Content;
    `description.en` empty; `repository` missing although some version is not planned, or rejected by the
    URL policy (§5.4); install folder (explicit or derived) not one safe path segment
    (`^[A-Za-z0-9._-]+$`, not `.`/`..`), or not derivable from the repository; `versions` empty;
  - version: does not parse or is listed twice; not planned and without `ref`, or with a `ref` that is not
    a safe git ref (no leading `-`, no `..`, no whitespace or control characters, none of `~ ^ : ? * [ \ "`;
    the same rule as `FUBotPackageIndex::IsSafeGitRef`, tested with the same vectors); `engine` or
    `requires[].version` is not a constraint; a `requires` entry with an empty name or a name that is not in
    this index; an `enginePlugins` entry that is not a plugin name;
  - a dependency cycle (non-optional requirements, using the latest, else the highest planned, version of
    each package);
  - recipe: empty or duplicate `id` (case-insensitive), empty `name.en` or `description.en`, no
    `packages`, a package that is not in this index;
  - replacement: empty or duplicate `legacy`, a `replacement` that is not in this index, `redirects` that
    is absolute or contains `..` or `\`.

  **Warnings** (exit 0): no index `name`; package without `layer` or `description`; missing `zh-CN` text
  (packages and recipes); a plain `http://` repository; two packages that install into the same folder;
  `ref` on a planned version; a recipe check id that is not known (only `rosBridge` is; unknown ids are
  ignored); unknown keys at any level.
- `Index/index.schema.json` is a JSON Schema (draft 2020-12) of the shape above.

### 1.1 Index sources

`settings.index.sources` is an ordered list. Defaults (both `builtin: true`; they cannot be removed or
disabled, only reordered: reading or storing the settings restores their id, name, location and
`enabled: true`):

1. `{ id: "ubot", name: "uBotCore", location: "<remote URL above>" }` — fetched over HTTPS
   (10 s timeout, honours `HTTPS_PROXY`/`HTTP_PROXY`; at most 5 redirects, followed only to `https://`
   targets, or to `http://` on the machine itself; a body over 8 MiB is refused). A successful fetch is parsed, then cached at
   `<cache>/index-ubot.json`. A failed fetch falls back to the cache (`fromCache: true`, `error` kept).
2. `{ id: "local", name: "Local copy", location: "<UBotCore>/Index/index.json" }` — the copy inside
   the UBotCore plugin installed in the open project (location shown project-relative, e.g.
   `Plugins/uBot/uBotCore/Index/index.json`). It is *unavailable*, not failed, when UBotCore is not
   installed in the project or ships no `Index/index.json`: the status row says why (`ok: false`, `error`
   set), nothing else happens and no problem is raised.

User sources (`builtin: false`; a blank location is dropped, a blank name shows the location):

- an `https://` URL, cached at `<cache>/index-<id>-<hash>.json` where `<hash>` is the FNV-1a hash of the
  location without credentials (so a source that keeps its id but gets another URL, or a positional
  `--index` id, never reads the old URL's cache). Plain `http://` is accepted only for the machine itself
  (`localhost`, `127.0.0.1`, `::1`, `*.localhost`), for local test servers;
- a file path: absolute, or relative to the project directory (a relative path needs an open project).
  Files and caches larger than 8 MiB are refused.

Status of a source (`IndexSourceStatus`): `ok` is true only when it loaded with no error, so a source served
from its cache after a failed fetch has `ok: false`, `fromCache: true`, `error` = the fetch error, and its
packages still count. Disabled sources have a row with `enabled: false` and nothing else.

If no source produced an index (every enabled source failed without a cache, is unavailable, or all are
disabled), the manager uses its embedded copy (`crates/ubot-core/assets/index.json`, a copy of uBotCore's
file, refreshed by `scripts/sync-index.mjs`); its status row is `{ id: "embedded", name: "Built-in copy" }`.
The embedded copy is not used while any other source loaded.

Merge: sources in list order; the **first** source that defines a package (by name, case-insensitive)
wins for that whole package; recipes merge by id the same way; replacements are the union, first wins
per legacy name. Failed sources are skipped, so the remote → local copy order *is* the fallback.

After every refresh with a project open, the manager writes the merged index (format 2, `name`
= "merged") to `<Project>/Saved/uBot/index.json` for the UE panel (§9), when its text changed.
Repository URLs in that file keep embedded credentials, because UE clones with them; everything the
manager shows or logs has them redacted (§5.4). The desktop app writes the file on every refresh; the
CLI only when it applies changes (`apply`, `recipe`), so its read-only commands leave the project untouched.

Fetching: `syncOnStart: true` → the app starts in two phases (§12): a quick rescan from local files and
caches without any network access (a source that was never cached shows as "not fetched yet"), then
`refresh(sync = true)`. Otherwise it starts from caches and fetches only the sources that have no cache
and did not fail earlier in this session. Rescans made by `plan` and `apply` never use the network. The
CLI does the equivalent of the second phase for every command except `index validate` and `engines`
(`sync = syncOnStart`, never with `--offline`).

## 2. Versions and constraints

Port the grammar of `uBotCore/Source/UBotCore/Public/UBotSemVer.h` exactly (read the header and
`Private/UBotSemVer.cpp`; reuse the vectors in `Private/Tests/UBotPackageTests.cpp` as Rust tests):
lenient version parse (`1`, `1.2`, `v1.2.3`, `-pre`, `+build` ignored), SemVer precedence, and
whitespace-separated comparators with `= >= > <= < ^ ~`, X-range partials, and upper bounds from
`^`/`~`/partials excluding that bound's prereleases. Do not use the `semver` crate's `VersionReq`
(different grammar).

## 3. Project model

- **Open a project** = an existing `.uproject` file. Name = file stem. Project dir = its parent.
- **Plugin discovery** (installed packages): search `<ProjectDir>/Plugins` and every entry of the
  `.uproject` `AdditionalPluginDirectories` (relative to the project dir). Recurse into directories;
  when a directory contains a `*.uplugin`, record it and do not descend further. Skip `Binaries`,
  `Intermediate`, `Saved`, `DerivedDataCache`, `.git`, `node_modules` (recursion depth at most 16,
  entries visited in path order). UE refuses two plugins with one name, so the first one found is the
  one kept. All discovered plugins are kept (for legacy detection and enable/disable); those whose
  descriptor has a top-level `"UBot"` object (or that cannot be parsed but mention one) are **uBot
  packages**.
- **Descriptor fields**: `VersionName` (installed version), `FriendlyName`, `Description`, `DocsURL`,
  `EnabledByDefault`, and in `"UBot"`: `Layer`, `Repository`, `Requires` (`[{Name, Version, Optional}]`),
  `Provides`, `Tags`, `EnginePlugins` (names; also accept the legacy key `ExternalRequires`).
  Requirements/provides of an *installed* package come from its descriptor; for a not-installed
  package from its latest (or planned) index version. Parsing is lenient (like uBotCore's
  `ParseDescriptor`): a malformed field is skipped and reported as a descriptor issue (missing or
  non-SemVer `VersionName`, missing or unknown `Layer`, a `Requires` entry without `Name`, …), which
  becomes a `descriptorInvalid` problem (§4); the package still shows up. A descriptor that is not a JSON
  object is the same problem with the parse error as detail.
- **Enabled** (UE rules): an entry in `.uproject` `Plugins` with `"Enabled"` decides. Otherwise the
  descriptor's `EnabledByDefault` if present. Otherwise project plugins (found under the project's
  `Plugins` or `AdditionalPluginDirectories`) are enabled and engine plugins are disabled.
- **Engine plugins**: names from `<EngineRoot>/Engine/Plugins/**/*.uplugin` (same recursion rule),
  cached per engine root. A required engine plugin counts as present, and as enabled or disabled by the
  rules above, also when a plugin of that name was copied into the project (any discovered project
  plugin, uBot or not). Without a resolved engine it cannot be checked and raises no problem.
- **`.uproject` editing**: keep key order and unknown fields (`serde_json` with `preserve_order`),
  keep existing entries' other fields (`TargetAllowList`, …), keep the file's indentation (tabs or
  N spaces, detect) and line endings (CRLF/LF, detect), keep a trailing newline if present. Enable =
  set/append `{ "Name": X, "Enabled": true }`; disable = set `"Enabled": false` (append if absent);
  remove = delete the entry. Keep a UTF-8 byte order mark if the file has one. Writes are atomic (temp
  file + rename; the settings, task history, caches and the project files the manager edits all go
  through one helper, which clears the read-only flag of a read-only target for the rename and sets it
  again afterwards, so the file keeps the state source control gave it). A plan whose steps would
  rewrite a read-only `.uproject` is refused up front (`plan.projectReadOnly`: check it out or clear the
  flag first).
- **Engine resolution** from `EngineAssociation`:
  - `"5.5"`-style: `C:\ProgramData\Epic\UnrealEngineLauncher\LauncherInstalled.dat` (JSON,
    `InstallationList[].{InstallLocation, AppName: "UE_5.5"}`), else registry
    `HKLM\SOFTWARE\EpicGames\Unreal Engine\5.5` → `InstalledDirectory`. `kind: "launcher"`.
  - `{GUID}`: registry `HKCU\Software\Epic Games\Unreal Engine\Builds` value named by the GUID → root. `kind: "source"`.
  - empty: walk up from the project dir looking for `Engine/Build/Build.version`. `kind: "source"`,
    `id` = the root path.
  - `settings.build.engineId` overrides (an id from `list_engines`, or an absolute engine root entered
    by the user, `kind: "custom"`); when it resolves to nothing the association is used instead.
  - Version = `<root>/Engine/Build/Build.version` → `Major.Minor.Patch`. Engine `id` = the
    association string (`"5.5"`, the GUID) or the root path for custom.
  - `list_engines` = all launcher installs + all registered source builds, each with a readable
    `Build.version`. Engines are found on Windows only (the registry and the Epic launcher file).
- **Build target** (`ProjectInfo.buildTarget`): `<Name>Editor` when UBT can resolve that target: the
  project has `Source/*.Target.cs`, or an enabled plugin lives outside `<ProjectDir>/Plugins` (an
  `AdditionalPluginDirectories` root: UBT then makes a temporary target in `Intermediate/Source` itself,
  a "hybrid" project; this is how DevHost builds `UBotDevEditor`), or a plan is about to clone into such a
  root, or UBT's temporary `Intermediate/Source/<Name>Editor.Target.cs` already exists. Otherwise (a
  content-only project whose plugins all sit in `<ProjectDir>/Plugins`) the stock `UnrealEditor` target
  is built with `-Project=`. A `<Name>Editor` build that fails with UBT's "find target rules file" message
  is retried once as `UnrealEditor` (§5.3). `hasCode` = `<ProjectDir>/Source` exists (informational only).
- **Editor running**: a process named `UnrealEditor.exe` (or `UnrealEditor-Cmd.exe`) whose command
  line contains the project path (case-insensitive, `/` and `\` equivalent). `close_editor` asks it
  to close gracefully (`taskkill /PID <pid>` without `/F`; the editor may prompt to save).
- **Git state** of an installed package directory: `isCheckout` = `<dir>/.git` exists; `ref` = exact
  tag (`git describe --tags --exact-match`), else branch (`git rev-parse --abbrev-ref HEAD`), else
  short SHA; `dirty` = `git status --porcelain --untracked-files=no --ignore-submodules=all` is
  non-empty. A package folder may have come with the project (a zip, a shared project), so its
  `.git/config` is untrusted data: every git command runs with `GIT_TERMINAL_PROMPT=0`,
  `GCM_INTERACTIVE=never`, `core.fsmonitor=false` and `advice.detachedHead=false` (fetch and checkout in
  an existing checkout also with `core.hooksPath` pointing at a folder that does not exist), and a
  repository whose configuration names a program or a place git would use (clean/smudge/process filters
  other than git-lfs's own, `diff.external`, `diff.*.textconv|command`, `merge.*.driver`,
  `core.sshCommand|gitProxy|askPass|alternateRefsCommand`, non-standard `credential.*.helper`,
  `url.*.insteadOf|pushInsteadOf`, `http.proxy`, `https.proxy`, `remote.*.proxy|uploadpack|receivepack|vcs`,
  `gpg.program|defaultKeyCommand`, `protocol.*`, `include.*`/`includeIf.*`,
  `lfs.customtransfer.*`/`lfs.extension.*`) is not queried further: the state shown is
  `isCheckout: true` without a `ref`, the reason (it names the key, never the value) is kept for
  planning, and `update` is refused (`plan.gitUnreadable`). The same holds for any other reason git
  cannot describe the checkout (git missing, "dubious ownership", a corrupt repository): `dirty` and
  `ref` are then unknown, never assumed clean. The git executable (`settings.install.git`) must name git
  itself: no quotes or control characters, no `cmd`/`powershell`/`pwsh`/`sh`/`bash`, no
  `.bat`/`.cmd`/`.ps1`.
- **Package identity** is case-insensitive everywhere; display the index casing (or descriptor
  casing for packages missing from the index).

## 4. Problems

Computed on every refresh. Only **enabled** packages produce requirement problems (an optional
requirement is a problem only when the dependency is installed at a version that does not satisfy it);
descriptor problems are raised for every installed uBot package. Ignored problem ids are stored per
project in settings (key: the absolute `.uproject` path, compared case-insensitively) and filtered out.

| code (`problem.*`) | id | severity | group | facts | fixes (first = primary) |
|---|---|---|---|---|---|
| `requiresVersion` {package, dep, range, installed} | `requires:<pkg>:<dep>` | warning | pkg | package = "Pkg 0.2.0", requires = "Dep ^0.2.0", installed = "Dep 0.1.0" | `fix.updateTo` {package: dep, version} (highest index version satisfying range, only if one exists) → `[update dep@version]`; `fix.disable` {package} |
| `requiresMissing` {package, dep, range} | same | warning | pkg | package, requires | `fix.install` {package: dep} (if available in index) → `[install dep]`; `fix.disable` |
| `requiresDisabled` {package, dep} | same | warning | pkg | package, requires | `fix.enable` {package: dep} → `[enable dep]`; `fix.disable` |
| `enginePluginMissing` {package, plugin} | `enginePlugin:<pkg>:<plugin>` | error | pkg | package, plugin, engine | `fix.ignore` |
| `enginePluginDisabled` {package, plugin} | same | warning | pkg | package, plugin | `fix.enable` {package: plugin} → `[enable plugin]` |
| `legacyConflict` {legacy, replacement} | `legacy:<legacy>` | warning | `<Name>.uproject` | legacy = "AgentSensorCore 0.5.0", replacement = "UBotSensor 0.1.0", redirects = path (if any) | `fix.disable` {package: legacy} → `[disable legacy]`; `fix.ignore` |
| `descriptorInvalid` {package, detail} | `descriptor:<pkg>` | warning | pkg | file = descriptor path | — |
| `dependencyCycle` {cycle} | `cycle:<first>` | error | pkg | — | — |
| `engineNotFound` {association} | `engine` | error | `<Name>.uproject` | — | `fix.openBuildSettings` → openSettings build |
| `indexSourceFailed` {source, error} | `index:<sourceId>` | `info` or `warning`, see below | the fixed string `Package Index` (the frontend shows "Package Index" / "Package 索引") | source = location | `fix.ignore` |
| session problems (§9.2) | `session:<id>` | from file | `package` from file | source = `Saved/uBot/status.json`, one fact per known param (`address`…), time = `session.lastPie`/`lastEditor`/`lastGame` | per code: `rosBridgeUnreachable` → `fix.openRosSettings` → openSettings ros |
| `rosBridgeUnreachable` {address} (also produced by the manager when `ros.checkOnRefresh` and UBotROS is enabled and a 2 s TCP connect fails) | `session:ros.bridge` / `ros:bridge` | info | `UBotROS` | address | `fix.openRosSettings` |

Scopes: session problems (from `status.json`) have `scope: "session"`, everything else `"packages"`.

**Index source problems.** One per source that failed to load. A source that is merely *unavailable*
(the local copy without an installed UBotCore, §1.1) raises none. The severity follows what is lost:
`info` when the source was served from its cache, or when it is built-in and the merged index still has
packages (from its cache, another source or the embedded copy); `warning` when a user-added source fails
without a cache to fall back to, or when no index data is available at all. They are about the indexes,
not about a package: whatever the source is called (the built-in source is named `uBotCore`, like a
package) they are never in a package's `problemIds`, and they are grouped under the fixed
`Package Index` title.

**Owners.** `PackageInfo.problemIds` holds the package's own requirement, engine plugin, descriptor and
cycle problems; the legacy conflicts it resolves (a conflict belongs to the *replacement* package although
its group is the project file); the manager's ROS check for `UBotROS`; and the session problems whose
`package` names it. Project (`engine`), index and unowned session problems belong to no package.

**Session problems** (from `status.json`, §9.2): `message` is `problem.<code>` with the file's `params`
when that is a known message code (§8), otherwise the file's `message` text as the `text` message
(§8). `group` is the file's `package`, or `<Name>.uproject` when it has none. Facts: `fact.source` =
`Saved/uBot/status.json`; one fact per param whose label `fact.<param>` exists (`address`, `plugin`, …); and
`fact.time` = the session's end (else start) time, with `valueMsg` = `session.lastPie|lastEditor|lastGame`.
The id is `session:<id>`; the manager's own ROS check uses `ros:bridge`, scope `packages`, group `UBotROS`.

## 5. Changes, plan, execution

### 5.1 Changes

`Change = { kind: install | update | enable | disable | uninstall, package, version? }`. The UI keeps
at most one change per package (a new one replaces the old; the planner does the same, the last one
wins). `enable`/`disable` also work for non-uBot plugins (legacy plugins, engine plugins) — they only
touch `.uproject`. `version` is read by `install` and `update` only; `null` means the latest index
version for both (the CLI is stricter, §10).

### 5.2 Planning (`plan(changes, options) -> Plan`)

Planning rescans the project without network access and resolves over the current installed set + the
merged index. Changes are processed in this order: disable and uninstall, update, install, enable.
`Plan.changes` echoes them, with the package casing and the version of an install filled in. With no
project open, any change is the error `plan.noProject`.

- **install X[@v]**: any plugin named X already in the project → `plan.alreadyInstalled`. X must exist
  in the index (`plan.notInIndex`) with a non-planned version (`plan.planned`; default latest; `v` must
  exist, else `plan.noVersion`). Its name, `ref`, repository URL and install folder must be safe
  (`plan.unsafeName`, `plan.unsafeRef`, `plan.unsafeUrl`). The package is cloned at the version's `ref`
  (§5.3) into `<ProjectDir>/<install.directory>/<folder>`, where `folder` is the index entry's `folder`
  (default: the last segment of the repository URL without `.git`). The install directory must be a
  plugin root of the project — `<ProjectDir>/Plugins` or an `AdditionalPluginDirectories` entry, or
  inside one — because UE and the manager find packages only there (`plan.notPluginDir` when it lies
  inside the project elsewhere, `plan.outsideProject` when it lies outside the project and outside every
  `AdditionalPluginDirectories` entry). An existing destination folder → `plan.folderExists`. A version
  whose `engine` constraint the project's engine does not satisfy → warning `plan.engineVersion` (not an
  error). For each non-optional requirement of that version, recursively: installed & satisfies &
  enabled → nothing; installed & satisfies & disabled → enable; installed & not satisfying → update to
  the highest index version that satisfies it *and* every other enabled dependent's constraint (a
  dependent that is itself about to be updated counts with its new requirements; when no version
  satisfies all of them the requirement of this package decides and the final check below names the
  dependents left unsatisfied), else `plan.noVersion`; not installed → install the highest satisfying
  index version. Required engine plugins → enable if not enabled (`plan.enginePluginMissing` if the
  engine lacks it; without an engine the entry is written unchecked). Then enable X.
- **update X[@v]** (also used for downgrades): X installed (`plan.notInstalled`), in the index
  (`plan.notInIndex`) with that version (`plan.noVersion`); a version equal to the installed one changes
  nothing. X must be a git checkout (`plan.notGit`), whose state git could read (`plan.gitUnreadable`,
  §3) and that is clean (`plan.dirty`). Warning `plan.onBranch` when the checkout is on a branch (it will
  be detached at the tag). A package this plan also clones is cloned at the new version instead.
  Requirements of the target version are resolved as for install when X is enabled.
- **enable X**: X is a discovered plugin or an engine plugin (`plan.notInstalled` otherwise); resolve
  requirements as for install (enable/install/update deps); engine plugins.
- **disable X**: enabled packages that require X (as a package or as an engine plugin), transitively,
  are disabled too (dependents first) with warning `plan.alsoDisable` {package, dependents}. A package
  the user disables or uninstalls is protected for the rest of the plan: a change that would need it
  again is `plan.breaksDependent`.
- **uninstall X**: like disable for dependents, then remove the `.uproject` entry and move the folder
  to the Recycle Bin (`trash` crate). Refuses folders outside the project dir (`plan.outsideProject`) and
  a folder that is a plugin root itself, which holds other plugins too (`plan.pluginRoot`).
- **Final check**: after all changes are applied to the virtual state, every enabled package whose own
  version or whose dependency's version the plan changed must have its requirements met by the final
  versions, whatever the order of changes and `requires` lists; an explicit update that ended at another
  version is explained by the dependents that turned it back. A violation is `plan.breaksDependent`
  {package, dependent, range}. A conflict that exists already and is not touched by the plan is left to
  the problems list (§4).
- Errors (`plan.*` Msgs) make the plan unappliable; warnings are shown and do not block.

Step order and groups: `download` (clone, checkout — dependencies first) → `project` (enable engine
plugins, then enable packages in dependency order, disable dependents-first, remove entries) → `build`
→ `after`. `build` is planned when something needs building — any clone, checkout, package enable or
engine plugin enable — and `options.build`; it needs the project's engine (`plan.noEngine` when
something needs building, `options.build` or `options.runTests` is set and no engine was found), a plain
target name and no `%` in the `.uproject` or engine path (`plan.unsafeProjectName`,
`plan.unsafeBuildPath`: `cmd.exe` would expand it). `after` holds `runTests` when `options.runTests` and
the plan cloned, checked out or enabled uBot packages (their names make the test filter), then
`openEditor` when `options.reopenEditor`, the editor was running or the plan has a build step, and the
plan has any step. The app shows the `after` group as "Finish" / "完成后". Steps are numbered from 1
(`id` = `s<n>`). `needsEditorClosed` is true for every step except `clone`.

### 5.3 Execution (tasks)

- One task at a time (second `apply` → error `task.busy`; so is opening another project while a task
  runs, which would mix up the project the task works on). `apply` re-plans and rejects with every plan
  error, one per line. Steps run sequentially on a background thread; each step streams its process
  output line by line (stdout + stderr, UTF-8 lossy, `\r` progress lines split, credentials in URLs
  redacted) to the task log. A failed step fails the task and skips the remaining steps; nothing is
  rolled back, except the partial clone below.
- Before a `needsEditorClosed` step, if the editor runs this project: when
  `settings.build.waitForEditor` the step becomes `waiting` and polls every second until the editor
  exits; otherwise the step fails with `task.editorRunning`.
- `clone`: `git clone --progress --branch <ref> -- <url> <dest>` (`--` before the URL). A `ref` that
  looks like a commit id (7 to 40 hex digits) is cloned without `--branch` and then checked out with
  `git checkout --detach`. The step fails when `<dest>` exists, and when the clone has no
  `<Package>.uplugin` at its root. On failure or cancel it removes the partially cloned folder (the
  folder did not exist before the step).
- `checkout`: refuses a checkout whose `.git/config` is untrusted (§3), then
  `git -C <dir> fetch --tags --no-recurse-submodules origin`, resolves `<ref>` itself (the tag
  `refs/tags/<ref>`, else `refs/remotes/origin/<ref>`, else whatever git makes of the name) and runs
  `git -C <dir> checkout --detach <commit>`.
- `enablePackage`, `disablePackage`, `enableEnginePlugin`: edit the `.uproject` (§3) and log the entry.
  `removePackage`: remove the entry, then move the folder to the Recycle Bin; it fails for a folder
  outside the project or one that is a plugin root. Tests and CI replace the Recycle Bin: the CLI deletes
  the folder permanently when `UBOT_MANAGER_PERMANENT_DELETE=1` (§10).
- `build`: `cmd.exe /S /C ""<EngineRoot>\Engine\Build\BatchFiles\Build.bat" <Target> Win64
  <Configuration> -Project="<uproject>" -WaitMutex -NoHotReloadFromIDE"`. Progress = last `[n/m]` seen in
  the output (exposed on the step as `progress: {done, total}`). When UBT answers that the target has no
  rules file and the target was not `UnrealEditor`, the step is rebuilt as `UnrealEditor` with the same
  `-Project=`, and its title changes accordingly (§3, build target). Building works on Windows only.
- `runTests`: `<EngineRoot>\Engine\Binaries\Win64\UnrealEditor-Cmd.exe "<uproject>" -unattended -nop4
  -nosplash -NullRHI -culture=en -stdout -FullStdOutLogOutput -ExecCmds="Automation RunTests <Pkg1>+<Pkg2>…; Quit"
  -TestExit="Automation Test Queue Empty"` with the names of the packages the task installed/updated/
  enabled (`-stdout` puts the log on the pipe the task reads; without it a failing run is only
  "exit code 255").
- `openEditor`: start `<EngineRoot>\Engine\Binaries\Win64\UnrealEditor.exe "<uproject>"` detached.
- Cancel kills the running child process tree (`taskkill /T /F /PID`), marks the step `canceled`
  and the remaining steps `skipped`. A step that is `waiting` for the editor is canceled the same way.
  Closing the app (and Ctrl+C in the CLI) cancels the running task; the app waits up to 10 s for it to
  clean up.
- Task history (last 30 tasks, each step log capped at 5000 lines) persists in `<data>/tasks.json`.
  Tasks that were still running when the manager exited are marked `failed` on the next load. A history
  file that cannot be read is copied to `tasks.json.bad-<unix time>` before it is replaced.
- When a task finishes the app refreshes the snapshot (without syncing) and emits it.
- When the plan came from a recipe with "save as recipe", or the Apply dialog's "Save as Recipe" is
  checked, and the task finished `done` while its project is still the open one, the resulting enabled
  uBot package set (all enabled uBot packages of the project, sorted by name) is saved as a user recipe
  (settings) with id `user-<slug of the name>` (lower-case letters and digits, other runs replaced by
  `-`; the same name replaces the earlier recipe).

### 5.4 URL policy (repository URLs from any index)

Allowed: `https://…`, `http://…` (warning), `ssh://…`, scp-like `user@host:path`, `file://…`, and
absolute local paths (for team mirrors and tests). Rejected: anything starting with `-`, containing
`::` transports (`ext::`, `fd::`; a `::` inside an IPv6 address is fine), control characters, quotes
or whitespace. Logged/displayed URLs, task logs, error texts and problem texts have embedded credentials
redacted (`https://user:token@host` → `https://***@host`; `ssh://git@host` keeps its plain user name,
`user:password` pairs never survive). The same rule is implemented in C++ by
`FUBotPackageService::IsRepositoryUrlAllowed`. Index *sources* have their own, stricter rule (§1.1).

## 6. Settings

File: `<config>/settings.json` where `<config>` = `%APPDATA%\AstreoX\uBot Manager`. `<data>` and
`<cache>` = `%LOCALAPPDATA%\AstreoX\uBot Manager` (`cache` subfolder for index caches). When the
environment variable `UBOT_MANAGER_HOME` is set (non-empty), `<config>` and `<data>` are that directory
(`settings.json`, `tasks.json`) and `<cache>` is its `cache` subfolder — tests, CI and smoke runs that start
the app or the CLI must set it. Rust unit tests do not need it: they build their `AppDirs` explicitly and
keep all fixtures in temporary folders under `%TEMP%`. Unknown keys are preserved; missing keys take
defaults:

```jsonc
{
  "language": "en",                 // "en" | "zh-CN"; first run: zh-CN when the OS UI language is Chinese
  "theme": "dark",                  // "dark" | "light" | "system"
  "index": { "sources": [/* §1.1 defaults */], "syncOnStart": true, "showPlanned": true },
  "install": { "directory": "Plugins/uBot", "git": "git", "usePrebuilt": false },
  "build": { "engineId": null, "configuration": "Development", "waitForEditor": true,
             "reopenEditor": true, "runTests": false },
  "ros": { "host": "127.0.0.1", "port": 8268, "checkOnRefresh": true },
  "recentProjects": [], "lastProject": null,
  "ignoredProblems": { "<abs .uproject path>": ["legacy:AgentSensorCore"] },
  "userRecipes": [ /* Recipe with user: true */ ]
}
```

ROS host/port are per project when UBotROS is installed: read from / written to
`<ProjectDir>/Config/DefaultUBotROS.ini`, section `[/Script/UBotROS.UBotROSSettings]`, keys
`BridgeHost`, `BridgePort` (edit only those lines; create the section/file if missing; values may be
quoted when read; a byte order mark and the line ending style are kept). That is the file UE reads:
`UUBotROSSettings` is declared `Config = UBotROS` with `DefaultConfig`, so a section of the same name in
`DefaultEngine.ini` has no effect. `get_settings` returns the ini's values where it has them;
`save_settings` writes the ini only when host or port differ from what `get_settings` returned, and then
leaves the app-level `ros.host`/`ros.port` as they were. Without UBotROS the app-level values are used.
The bridge check of a refresh (`ros.checkOnRefresh`, §4) uses the same address.

Reading is tolerant: a value of the wrong type resets only its own key (a team index source survives a bad
`syncOnStart`), invalid elements of a list are dropped, and the settings are normalized — the built-in
index sources are restored (§1.1), blank install directory / git / ROS host / port 0 take their defaults,
a blank `engineId` becomes `null`, `recentProjects` has no blanks or case-insensitive duplicates and at
most 10 entries (newest first). `install.usePrebuilt` is reserved and has no effect today (the dialog
shows it disabled). The first run (no `language` key) picks `zh-CN` when the OS UI language is Chinese.
`lastProject` is reopened at startup only if the file still exists. `open_project` is the only thing that
changes `recentProjects`/`lastProject`: `save_settings` ignores what the caller sends for them, so a copy
of the settings loaded earlier cannot revert them. The file is written with 4-space indentation and a
trailing newline. A settings file that exists but cannot be used (not valid JSON, not an object, locked by
a sync tool; UTF-8 with or without BOM and UTF-16 with BOM are accepted) leaves the defaults in effect
and is copied to `settings.json.bad-<unix time>` before the next save replaces it.

Language changes take effect after restart (the dialog says so). Theme applies immediately.

## 7. API contract (Tauri commands and events)

TypeScript is the reference. Rust mirrors it with `#[serde(rename_all = "camelCase")]` structs,
string enums with the exact literal values below, `Option<T>` → `null` (always present, never
omitted), and internally tagged unions (`#[serde(tag = "kind")]`). Tauri command names are
snake_case in Rust and called with camelCase argument names from JS.

```ts
export type Lang = 'en' | 'zh-CN';
export type Layer = 'Foundation' | 'Capability' | 'Adapter' | 'Content' | 'Unknown';
export interface LocalizedText { en: string; 'zh-CN': string | null }
/** Message for the UI: `code` is a key under `msg.` in the locale files; params are plain strings. */
export interface Msg { code: string; params: Record<string, string> }

export interface EngineInfo { id: string; version: string; root: string; kind: 'launcher' | 'source' | 'custom' }
export interface ProjectInfo {
  path: string; name: string; dir: string; engineAssociation: string;
  engine: EngineInfo | null; hasCode: boolean; buildTarget: string;
  editor: { running: boolean; pid: number | null };
}

export type PackageState = 'enabled' | 'disabled' | 'notInstalled' | 'planned';
export interface Requirement {
  name: string; constraint: string; optional: boolean; kind: 'package' | 'enginePlugin';
  status: 'satisfied' | 'versionMismatch' | 'notInstalled' | 'disabled' | 'missing';
  installedVersion: string | null;
}
export interface GitState { isCheckout: boolean; ref: string | null; dirty: boolean }
export interface PackageInfo {
  name: string; friendlyName: string; description: LocalizedText; layer: Layer; state: PackageState;
  installedVersion: string | null; latestVersion: string | null;
  versions: string[];               // non-planned index versions, newest first
  plannedVersion: string | null;
  updateAvailable: boolean;
  location: string | null;          // project-relative, '/' separators ("Plugins/uBot/uBotSensor", "../Plugins/uBot/uBotCore")
  git: GitState | null;             // null when not installed
  repository: string | null; webUrl: string | null; docsUrl: string | null;
  indexSource: string | null;       // id of the source that defined it
  requires: Requirement[]; provides: string[]; tags: string[];
  problemIds: string[];
}
export interface PluginInfo { name: string; enabled: boolean; location: string; version: string | null; isUBot: boolean }

export interface IndexSourceStatus {
  id: string; name: string; location: string; builtin: boolean; enabled: boolean;
  ok: boolean; fromCache: boolean; error: string | null; packages: number; fetchedAt: string | null; // RFC 3339
}
export interface Recipe {
  id: string; name: LocalizedText; description: LocalizedText; packages: string[]; checks: string[];
  available: boolean; user: boolean;
}

export type Severity = 'error' | 'warning' | 'info';
export type SettingsSection = 'appearance' | 'index' | 'install' | 'build' | 'ros';
export interface ProblemFact { label: Msg; value: string; valueMsg: Msg | null }
export type FixAction =
  | { kind: 'changes'; changes: Change[] }
  | { kind: 'ignore' }
  | { kind: 'openSettings'; section: SettingsSection };
export interface Fix { label: Msg; primary: boolean; action: FixAction }
export interface Problem {
  id: string; scope: 'packages' | 'session'; group: string; severity: Severity;
  message: Msg; facts: ProblemFact[]; fixes: Fix[];
}

export interface SessionStatus {        // parsed Saved/uBot/status.json, see §9.2
  kind: 'PIE' | 'Editor' | 'Game'; startedAt: string; endedAt: string | null; engineVersion: string;
}

export interface Snapshot {
  project: ProjectInfo | null;
  packages: PackageInfo[];              // index + installed, sorted: layer order, then name
  plugins: PluginInfo[];                // every discovered project plugin
  problems: Problem[];                  // ignored ones removed
  index: IndexSourceStatus[];
  recipes: Recipe[];                    // index recipes then user recipes
  session: SessionStatus | null;
  syncing: boolean;
}

export type ChangeKind = 'install' | 'update' | 'enable' | 'disable' | 'uninstall';
export interface Change { kind: ChangeKind; package: string; version: string | null }
export type StepGroup = 'download' | 'project' | 'build' | 'after';
export type StepKind = 'clone' | 'checkout' | 'enablePackage' | 'disablePackage' | 'enableEnginePlugin'
  | 'removePackage' | 'build' | 'runTests' | 'openEditor';
export interface PlanStep { id: string; n: number; group: StepGroup; kind: StepKind; title: Msg; detail: Msg; needsEditorClosed: boolean }
export interface ApplyOptions { build: boolean; reopenEditor: boolean; runTests: boolean }
export interface Plan {
  changes: Change[]; steps: PlanStep[]; needsBuild: boolean; editorRunning: boolean;
  errors: Msg[]; warnings: Msg[];
}

export type StepStatus = 'pending' | 'waiting' | 'running' | 'done' | 'failed' | 'skipped' | 'canceled';
export interface TaskStep {
  id: string; title: Msg; status: StepStatus; startedAt: string | null; durationMs: number | null;
  progress: { done: number; total: number } | null; logLines: number;
}
export interface Task {
  id: string; title: Msg; project: string; status: StepStatus; startedAt: string;
  durationMs: number | null; steps: TaskStep[]; error: string | null; changes: Change[];
}

export interface Settings {
  language: Lang; theme: 'dark' | 'light' | 'system';
  index: { sources: IndexSourceConfig[]; syncOnStart: boolean; showPlanned: boolean };
  install: { directory: string; git: string; usePrebuilt: boolean };
  build: { engineId: string | null; configuration: 'Development' | 'DebugGame'; waitForEditor: boolean; reopenEditor: boolean; runTests: boolean };
  ros: { host: string; port: number; checkOnRefresh: boolean };
  recentProjects: string[]; lastProject: string | null;
}
export interface IndexSourceConfig { id: string; name: string; location: string; enabled: boolean; builtin: boolean }
export interface EnvCheck { id: 'git' | 'vs2022' | 'engine' | 'rosBridge'; ok: boolean | null; detail: string | null }
```

Notes on the types:

- `Snapshot.packages` has every package of the merged index plus every installed uBot plugin the index does
  not list (its layer, description and requirements then come from its descriptor), sorted by layer
  (Foundation, Capability, Adapter, Content, Unknown), then by name. For a listed package the index
  casing of the name wins. `PackageInfo.repository` and `webUrl` have credentials removed.
- `IndexSourceStatus.ok` is false for a source that is served from its cache (`fromCache: true`), and for
  one that is unavailable; `packages` still counts what was loaded (§1.1).
- `Problem.group` is display text: a package name, the `.uproject` file name, or the fixed string
  `Package Index` for index source problems (§4). `ProblemFact.valueMsg`, when set, is shown instead of
  `value`.
- `Plan.changes` is the changes as resolved by the planner (§5.2). `PlanStep.group` `after` is displayed
  as "Finish" / "完成后"; see §5.2.
- `Msg.params` are plain strings; the frontend fills `{name}` placeholders of the locale template.

Commands (all async on the JS side). Errors reject with an English string. Plan and task rejections are
rendered from the `plan.*`/`task.*` templates of §8, one message per line; the frontend recognises those
lines and shows them in the UI language, and shows everything else (git output, internal errors) as is:

| command | args | returns |
|---|---|---|
| `get_snapshot` | — | `Snapshot` (the cached one with `editor` and `syncing` re-checked; scans once when there is none; during app startup it waits for the quick scan, §12) |
| `refresh` | `{ sync: boolean }` | `Snapshot` (rescan project; `sync` re-fetches remote sources; a `sync` request that arrives while the startup sync runs, or within 30 s after it, is answered by it, §12) |
| `open_project` | `{ path: string }` | `Snapshot` (an existing `.uproject`; adds to recent projects, sets lastProject; rejected with `task.busy` while a task runs; all or nothing: on failure the previous project stays open) |
| `plan` | `{ changes: Change[], options: ApplyOptions }` | `Plan` (rescans locally, never uses the network) |
| `apply` | `{ changes: Change[], options: ApplyOptions, title: Msg \| null, saveRecipe: string \| null }` | `string` task id (re-plans internally; rejects when the plan has errors) |
| `cancel_task` | `{ taskId: string }` | `void` (a finished task is a no-op; an unknown id rejects) |
| `list_tasks` | — | `Task[]` newest first |
| `get_task_log` | `{ taskId: string, stepId: string \| null }` | `string[]` (`null` step = task summary lines) |
| `close_editor` | — | `void` (`plan.noProject` without a project; nothing happens when no editor runs it) |
| `ignore_problem` | `{ id: string }` | `Snapshot` (for the open project) |
| `get_settings` | — | `Settings` (ROS host/port from the project's ini where it has them, §6) |
| `save_settings` | `{ settings: Settings }` | `Snapshot` (stores the normalized settings, §6, then refreshes without syncing) |
| `test_git` | `{ path: string }` | `{ ok: boolean, version: string \| null, error: string \| null }` |
| `test_ros` | `{ host: string, port: number }` | `{ ok: boolean, error: string \| null }` (TCP connect, 2 s in all; every resolved address is tried, IPv4 first) |
| `list_engines` | — | `EngineInfo[]` |
| `check_environment` | `{ checks: string[] }` | `EnvCheck[]` (one per known requested id, in request order: `git`, `vs2022` via vswhere (any VS 2022 edition, Build Tools included), `engine`; `rosBridge` → `ok: null`, not checked; unknown ids are ignored) |
| `recipe_changes` | `{ recipeId: string \| null, packages: string[] }` | `Change[]` (install missing, enable disabled; nothing else) |

Events (Tauri `emit` to all windows):

- `ubot://snapshot` → `Snapshot` after background refreshes (the quick scan and the full refresh at
  startup, task finished, project opened from a second instance, sync finished).
- `ubot://task` → `Task` whenever a task or step status/progress changes.
- `ubot://task-log` → `{ taskId: string; stepId: string; lines: string[] }` (batched ≤ every 100 ms).

## 8. Message codes

Every `Msg.code` the backend emits must exist in both `app/src/i18n/en.json` and `zh-CN.json`
under `msg.`. The locale files are two-level (`{ "msg": { "step.clone.detail": "…" } }`), because codes
contain dots. Templates use `{name}` placeholders. Rust code may only emit codes from this list
(`crates/ubot-core/src/msg.rs` holds the English templates the CLI renders with); a new code needs both
translations added in the same change. The frontend test `i18n.test.ts` checks that every code of this
table is in both files with the parameters listed here.

| code | en | zh-CN |
|---|---|---|
| problem.requiresVersion | Requires {dep} {range}, but {installed} is installed. | 需要 {dep} {range}，当前安装的是 {installed}。 |
| problem.requiresMissing | Requires {dep} {range}, which is not installed. | 需要 {dep} {range}，但尚未安装。 |
| problem.requiresDisabled | Requires {dep}, which is disabled. | 需要 {dep}，但它已禁用。 |
| problem.enginePluginMissing | Needs engine plugin {plugin}, which this engine does not have. | 需要引擎插件 {plugin}，当前引擎中没有。 |
| problem.enginePluginDisabled | Needs engine plugin {plugin}, which is disabled. | 需要引擎插件 {plugin}，但它已禁用。 |
| problem.legacyConflict | {legacy} and {replacement} are both enabled. | {legacy} 和 {replacement} 同时启用。 |
| problem.descriptorInvalid | The descriptor of {package} has errors: {detail} | {package} 的描述文件有误：{detail} |
| problem.dependencyCycle | Dependency cycle: {cycle}. | 依赖循环：{cycle}。 |
| problem.engineNotFound | Engine {association} for this project was not found. | 找不到此项目使用的引擎 {association}。 |
| problem.indexSourceFailed | Could not load {source}: {error} | 无法加载 {source}：{error} |
| problem.rosBridgeUnreachable | Could not connect to ubot_ros_bridge at {address}. | 无法连接 {address} 上的 ubot_ros_bridge。 |
| fact.package / requires / installed / legacy / replacement / redirects / source / address / time / plugin / engine / file | Package / Requires / Installed / Legacy plugin / Replaced by / Redirects / Source / Address / Time / Plugin / Engine / File | Package / 依赖 / 已安装 / 旧插件 / 替代为 / 重定向 / 来源 / 地址 / 时间 / 插件 / 引擎 / 文件 |
| session.lastPie / lastEditor / lastGame | last PIE session / last editor session / last game session | 上次 PIE 运行 / 上次编辑器运行 / 上次游戏运行 |
| fix.updateTo {package, version} | Update {package} to {version} | 将 {package} 更新到 {version} |
| fix.install {package} | Install {package} | 安装 {package} |
| fix.enable {package} | Enable {package} | 启用 {package} |
| fix.disable {package} | Disable {package} | 禁用 {package} |
| fix.ignore | Ignore | 忽略 |
| fix.openRosSettings | Open ROS Settings | 打开 ROS 设置 |
| fix.openBuildSettings | Open Build Settings | 打开编译设置 |
| step.clone {package, version} | Clone {package} {version} | 克隆 {package} {version} |
| step.clone.detail {repo, dest} | {repo} → {dest} | {repo} → {dest} |
| step.checkout {package, version} | Check out {package} {version} | 检出 {package} {version} |
| step.checkout.detail {from, to} | {from} → {to} | {from} → {to} |
| step.enablePackage {package} | Enable {package} | 启用 {package} |
| step.disablePackage {package} | Disable {package} | 禁用 {package} |
| step.enableEnginePlugin {plugin} | Enable {plugin} | 启用 {plugin} |
| step.removePackage {package} | Remove {package} | 移除 {package} |
| step.removePackage.detail {dest} | {dest} → Recycle Bin | {dest} → 回收站 |
| step.detail.project {file} | {file} | {file} |
| step.detail.requiredBy {package} | required by {package} | {package} 需要 |
| step.build {target} | Build {target} | 编译 {target} |
| step.build.detail {platform, configuration} | {platform} · {configuration} | {platform} · {configuration} |
| step.runTests | Run package tests | 运行 Package 测试 |
| step.runTests.detail {filter} | {filter} | {filter} |
| step.openEditor | Reopen Unreal Editor | 重新打开 UE 编辑器 |
| step.openEditor.detail {project} | {project} | {project} |
| plan.notInIndex {package} | {package} is not in any package index. | 所有 Package 索引中都没有 {package}。 |
| plan.planned {package} | {package} is planned and cannot be installed yet. | {package} 还在规划中，暂时无法安装。 |
| plan.noVersion {package, range} | No version of {package} matches {range}. | {package} 没有符合 {range} 的版本。 |
| plan.notInstalled {package} | {package} is not installed. | {package} 尚未安装。 |
| plan.alreadyInstalled {package} | {package} is already installed. | {package} 已经安装。 |
| plan.notGit {package} | {package} is not a git checkout; reinstall it to update. | {package} 不是 git 检出，需要重新安装才能更新。 |
| plan.dirty {package} | {package} has local changes. | {package} 有本地修改。 |
| plan.onBranch {package, branch} | {package} is on branch {branch}; it will be detached at the new version. | {package} 在分支 {branch} 上，更新后会切换到新版本的标签。 |
| plan.breaksDependent {package, dependent, range} | {dependent} requires {package} {range}. | {dependent} 需要 {package} {range}。 |
| plan.alsoDisable {package, dependents} | Disabling {package} also disables {dependents}. | 禁用 {package} 会同时禁用 {dependents}。 |
| plan.enginePluginMissing {plugin} | The engine has no plugin named {plugin}. | 引擎中没有名为 {plugin} 的插件。 |
| plan.folderExists {dest} | {dest} already exists. | {dest} 已存在。 |
| plan.unsafeUrl {url} | Refusing repository URL {url}. | 拒绝使用仓库地址 {url}。 |
| plan.noEngine | No Unreal Engine was found for this project. | 找不到此项目的 UE 引擎。 |
| plan.engineVersion {package, version, range, engine} | {package} {version} supports UE {range}, this project uses {engine}. | {package} {version} 支持 UE {range}，此项目使用的是 {engine}。 |
| plan.outsideProject {path} | {path} is outside the project. | {path} 不在项目目录内。 |
| plan.pluginRoot {path} | {path} is a plugin folder that holds more than this package. | {path} 是一个插件目录，里面除了此 Package 还有其他内容。 |
| plan.notPluginDir {path} | {path} is not inside the project's Plugins folder or one of its AdditionalPluginDirectories. | {path} 不在项目的 Plugins 文件夹或 AdditionalPluginDirectories 之内。 |
| plan.gitUnreadable {package, error} | Cannot read the git state of {package}: {error} | 无法读取 {package} 的 git 状态：{error} |
| plan.unsafeRef {package, ref} | Refusing git ref {ref} of {package}. | 拒绝使用 {package} 的 git 引用 {ref}。 |
| plan.unsafeName {package} | Refusing package name {package}. | 拒绝使用 Package 名称 {package}。 |
| plan.unsafeProjectName {name} | The project name {name} cannot be built; use only letters, digits and underscores in the .uproject file name. | 无法构建项目名 {name}；.uproject 文件名只能使用字母、数字和下划线。 |
| plan.unsafeBuildPath {path} | {path} contains a % character, which the Windows build scripts would expand. | {path} 含有 % 字符，Windows 构建脚本会对其展开。 |
| plan.projectReadOnly {file} | {file} is read-only; check it out or clear the read-only flag first. | {file} 为只读；请先签出它或清除只读属性。 |
| plan.noProject | No project is open. | 尚未打开项目。 |
| task.apply {project} | Apply Changes · {project} | 应用更改 · {project} |
| task.recipe {recipe, project} | Apply Recipe {recipe} · {project} | 应用 Recipe {recipe} · {project} |
| task.install {package, version} | Install {package} {version} | 安装 {package} {version} |
| task.busy | Another task is running. | 另一个任务正在运行。 |
| task.editorRunning | Unreal Editor is running this project. | UE 编辑器正在运行此项目。 |
| text {text} | {text} | {text} |

`text` is for text that has no code of its own: its template is only its parameter in every language. The
manager uses it for the `message` of a session problem whose `code` it does not know (§4).
`step.detail.project` is only the detail of steps that edit the project file.

## 9. uBotCore / uBotROS changes (UE 5.5)

### 9.1 Index in UE

- `Index/index.json` replaces `Resources/PackageIndex.json` (delete the old file). Add
  `+/Index/...` handling to `Config/FilterPlugin.ini` so packaged plugins keep it.
- `FUBotPackageIndex` parses **format 2** only (format 1 → clear error). From each package:
  `Version` = latest non-planned version, `AvailableVersions` (new, newest first), `bPlanned` (new),
  `Requires`/`Provides`/`ExternalRequires` (= `enginePlugins`) from the latest (or planned) version,
  `Description` = the editor culture's text (`zh*` → `zh-CN` when present, else `en`).
- `LoadConfiguredIndex`: `<Project>/Saved/uBot/index.json` (written by the manager) when it exists,
  otherwise `<UBotCore>/Index/index.json`; then `UUBotCoreSettings::AdditionalPackageIndexFiles`.
- Descriptor `"UBot"` blocks: read `EnginePlugins`, still accept `ExternalRequires`.
- The commandlet (`-run=UBotPackage`) and `FUBotPackageService` stay and keep working on format 2.
  Update their tests.

### 9.2 Runtime status (`Saved/uBot/status.json`)

New runtime API in UBotCore (`UBotRuntimeStatus.h`): a game-thread singleton where packages publish
status items and problems; `OnChanged` delegate; sessions begin/end. In the editor a session is a PIE
session (`FEditorDelegates::PostPIEStarted` / `EndPIE`); in a game build, from the first game world
`BeginPlay` to its teardown. At session end (and on module shutdown during a session) it writes:

```jsonc
{
  "formatVersion": 1,
  "writtenAt": "2026-10-10T08:00:00Z",
  "session": { "kind": "PIE", "startedAt": "...", "endedAt": "..." },
  "engineVersion": "5.5.4",
  "packages": [ { "name": "UBotCore", "version": "0.1.0", "enabled": true } ],
  "extensionPoints": [ { "id": "UBotROS.SensorPublisher", "implementations": 7 } ],
  "runtime": [ { "id": "ros.bridge", "label": "ROS Bridge", "value": "connected",
                 "detail": "127.0.0.1:8268", "severity": "ok" } ],      // ok | info | warning | error
  "problems": [ { "id": "ros.bridge", "package": "UBotROS", "severity": "info",
                  "code": "rosBridgeUnreachable", "params": { "address": "127.0.0.1:8268" },
                  "message": "Could not connect to ubot_ros_bridge at 127.0.0.1:8268." } ]
}
```

The manager maps `code` to `msg.problem.<code>` when that key exists, otherwise shows `message` (as the
`text` message, §8). It reads the file on every rescan and ignores one that is missing, has another
`formatVersion`, or whose `session.kind` is not `PIE`, `Editor` or `Game`; a problem entry without an
`id` is skipped, a severity other than `error`/`warning` counts as `info`, and a `params` value that is
not a string is taken as its JSON text. `Snapshot.session` carries the session part (§7). UBotROS publishes `ros.bridge` (connecting / connected / disconnected + address) and reports
`rosBridgeUnreachable` after the first failed connection attempt in a session; it clears the problem
once connected.

### 9.3 Editor panel (replaces `SUBotPackageManager`)

Delete the Slate package manager tab. Add a nomad tab **uBot** (Window menu, and a status-bar entry
in `LevelEditor.StatusBar.ToolBar` reading `uBot: 3 packages · 2 problems` / `uBot：3 个 Package ·
2 个问题` that opens the tab). Layout and copy: `UEPanel.dc.html` / `UEPanelZh.dc.html` and their
screenshots, built with standard `FAppStyle` widgets:

- Toolbar: **Open uBot Manager** button.
- Collapsible sections: **Loaded Packages** (name, version, `· update to x` from the index),
  **Extension Points** (id, `N implementations`), **Runtime** (status items from §9.2, sim time from
  the clock subsystem while PIE runs, fixed time step setting), **Problems** (registry problems,
  legacy conflicts).
- Open uBot Manager: executable from `UUBotEditorSettings::ManagerExecutable` (new per-user editor
  settings, `EditorPerProjectUserSettings`), else `HKCU\Software\AstreoX\uBot Manager` value
  `Executable`, else `%LOCALAPPDATA%\uBot Manager\ubot-manager.exe`; launched detached with
  `--project "<absolute .uproject>"`. Not found → notification with a hyperlink to
  `https://github.com/AstreoX/uBotManager/releases`.
- Localization: LOCTEXT in code; Simplified Chinese registered at module startup with
  `FTextLocalizationManager::RegisterPolyglotTextData` for culture `zh-Hans`, wording from
  `UEPanelZh.dc.html` and `Terms.dc.html`. The panel follows the editor language.

## 10. CLI (`ubot`)

```
ubot [--project <.uproject>] [--index <file|url>]... [--offline] [--json] <command>
  status                      packages and problems
  info <package>
  problems
  plan <change>...            change = install:Name[@ver] | update:Name@ver | enable:Name | disable:Name | uninstall:Name
  apply <change>... [--no-build] [--run-tests] [--reopen-editor] [--yes]
  recipes
  recipe <id> [--no-build] [--yes]
  engines
  index validate <file>       §1 validation; exit 1 on errors
  index show
```

The CLI uses the same core, settings, caches and task history as the app (§6).

- `--index` replaces the configured sources (tests, CI). Each value is a URL or a file; a relative file is
  taken from the current directory, not the project's. The sources are named `cli-1`, `cli-2`, …. One that
  cannot be loaded (and is not served from its cache) is an error for every command except `index show`.
- `--offline` uses caches only.
- `--project` opens that project, which also adds it to the recent projects and makes it `lastProject`
  (the app starts on it next time). Without `--project` the CLI uses `settings.lastProject`, if the file
  still exists; with no project at all `status` says so and the commands that change a project fail with
  `plan.noProject`.
- Every command except `index validate` and `engines` refreshes first, like the app (`sync =
  settings.index.syncOnStart`, never with `--offline`). Read-only commands do not write
  `<Project>/Saved/uBot/index.json`; only `apply` and `recipe` do (§1.1).
- Output is English plain text (message templates rendered with the English table), or JSON with
  `--json`: `status` the `Snapshot`, `info` `{ package, problems }`, `problems` the `Problem[]`,
  `plan` the `Plan` (so does `apply` when it stops before running anything), `apply`/`recipe` the finished
  `Task`, `recipes` the `Recipe[]`, `engines` the `EngineInfo[]`, `index validate` `{ ok, errors, warnings }`,
  `index show` `{ sources, index }` (repository URLs without credentials).
- Changes are `kind:Name[@version]`, the kind in any case. Only `install` and `update` take a version, and
  `update` needs one (`update:UBotSensor@0.2.0`): unlike the app, whose `Change.version = null` means
  "latest", the CLI does not guess.
- `plan` plans with `build: true` and exits 1 when the plan has errors.
- `apply` prints the plan first. Errors → exit 1. No steps → exit 0. Without `--yes` it asks
  `Apply these changes? [y/N]` (prompt on stderr, answer on stdin; end of input means no, exit 1). It then
  prints `[n/m] <step title>: <status> (<seconds>)` per step transition and the step output indented.
  Options `--no-build`, `--run-tests`, `--reopen-editor` (all off by default; the CLI does not read
  `build.runTests`/`build.reopenEditor` from the settings). Exit code 0 when the task is `done`, 130 when it
  was canceled, 1 when it failed. Ctrl+C and Ctrl+Break cancel the task, like Stop in the app (a second one
  exits at once with 130).
- `recipe <id>` is `apply` for the changes of `recipe_changes` (install the missing packages of the recipe,
  enable the disabled ones; §7), with the task title `task.recipe`; only `--no-build` and `--yes`. A recipe
  that is not available (a package is planned or not in any index) is refused; a recipe that has nothing
  to change prints "Every package of <recipe> is installed and enabled." and exits 0.
- `index validate <file>` needs no project: it prints `error:`/`warning:` lines and a summary, exit code 1
  when there are errors.
- `UBOT_MANAGER_HOME` moves settings, history and caches (§6). `UBOT_MANAGER_PERMANENT_DELETE=1` makes
  `uninstall` delete the package folder for good instead of moving it to the Recycle Bin; tests and CI set
  it (the app has no such switch).

## 11. Frontend (`app/`)

Vite + React + TypeScript (strict), no UI framework, plain CSS with theme tokens. Fonts bundled
locally (`@fontsource-variable/inter`, `@fontsource/jetbrains-mono`); CJK falls back to system fonts.
Font stacks: UI `Inter, 'Noto Sans SC', 'Microsoft YaHei UI', 'PingFang SC', sans-serif`; mono
`'JetBrains Mono', 'Noto Sans SC', 'Microsoft YaHei UI', monospace`. `<html lang>` follows the language.
Rendered CJK text differs slightly from the web design, which was drawn with a web font: the glyphs come
from the system fallback (Microsoft YaHei UI, or Noto Sans SC when it is installed).

Match the design screenshots pixel-for-pixel where data allows (sizes, colors, spacing, icons —
copy the inline SVG paths from the `.dc.html` files). Rules from the design reviews that must hold:
JetBrains IDE feel; no capsule/pill shapes; no sidebar besides the tool-window strip; no verbose or
explanatory copy; "Package" stays English in both languages; header shows only the logo, "uBot
Manager", Recipes, Refresh, and window controls (no project name); refresh is an icon button (spins
while syncing; tooltip shows index status); Settings (gear) in the strip; buttons opening dialogs end
with "…"; dialog buttons ≥ 72 px wide, toolbar buttons ≥ 56 px; Chinese typography rules from
`Terms.dc.html` (spaces between CJK and Latin/digits, full-width punctuation in sentences, no period
on buttons/labels, measure words "4 个步骤", "1 项待应用", no italics, weight ≤ 600, ≥ 12 px).

Structure:

- `src/api/types.ts` (§7 verbatim), `src/api/index.ts` (`api` object: Tauri `invoke`/`listen` when
  `'__TAURI_INTERNALS__' in window`, otherwise the mock), `src/api/mock.ts` (in-memory backend with
  the design scenario: project MyRobotSim; UBotCore 0.1.0, UBotSensor 0.1.0 → 0.1.1, UBotEnvironment
  0.2.0 with the `requiresVersion` problem, UBotROS 0.1.0, UBotCombat disabled, UBotVehicles /
  UBotAerial / UBotMCP not installed, UBotLearning planned, AgentSensorCore legacy conflict, ROS
  session problem; a running task that streams fake log lines and progress).
- `src/i18n/{en,zh-CN}.json` + `t(key, params)`; `tm(msg)` renders a `Msg`. Missing zh key → en →
  the key itself. The files are two-level (section → rest of the dotted key → text, §8), `.one`/`.other`
  suffixes pick singular and plural (`tn`). The built-in index sources are shown by their `source.*`
  texts ("uBotCore", "Local copy" / "本地副本", "Built-in copy" / "内置副本"), user sources by their name.
- `src/theme.css`: dark tokens from the design; light tokens in the JetBrains Light palette; `system`
  follows `prefers-color-scheme`.
- Views (tool-window strip): Packages (list + details), Graph, Problems; Tasks opens from the status
  bar; Settings (gear) opens the Settings dialog. Dialogs: Apply Changes (`Plan.dc.html`), Apply
  Recipe (`Welcome.dc.html`), Settings (`Settings.dc.html`). Status bar: breadcrumb, problem count
  (packages scope; errors and warnings, `info` problems are not counted), update count, pending-changes
  text with Apply… / Discard, Tasks; while a task runs the Tasks view footer shows step text, a progress
  bar, `n/m` and a cancel button.
- Packages list: two groups, "Installed" and "uBot Index" / "uBot 索引" (a fixed title; the `name` of
  the index is not used), ordered like `Snapshot.packages`; planned packages are listed (and drawn in the
  Graph) only while `index.showPlanned` is set. The problem group of index source problems is the fixed
  string `Package Index`, shown as "Package Index" / "Package 索引" (§4).
- Apply Changes: the steps appear under Download / Project / Build / Finish (`download`, `project`,
  `build`, `after`; "完成后" in Chinese; Finish holds `runTests` and `openEditor`). The options are "Reopen
  editor after build" (starts from `build.reopenEditor`) and "Save as recipe" with a name; the app always
  plans with `build: true`, and `runTests` starts from `build.runTests`, which has no checkbox here. While
  the dialog is open the app asks for the editor state every 2 s and re-plans when it changes ("Close
  Editor" asks the editor to close). Recipes (Apply Recipe) offer the recipe's packages with `UBotCore`
  locked on, run the environment checks (`vs2022`, `git`, plus the recipe's `checks`) and end in Review
  Changes…, which opens Apply Changes for `recipe_changes`.
- Problems: two tabs, Packages and Last Session (the `scope`s), grouped by `Problem.group`; the fixes of
  a `changes` action are queued as pending changes (reviewed through Apply…), `ignore` hides the problem
  for the project, `openSettings` opens the Settings dialog at that section.
- Settings dialog: sections Appearance, Package Index, Installation, Build, ROS Bridge with a search box;
  OK / Cancel / Apply. Theme previews immediately and Cancel restores it; the language needs a restart;
  built-in index sources can be reordered but not removed or disabled; "Use prebuilt binaries" is shown
  disabled (reserved, §6). The Installation and ROS Bridge pages test git / the bridge when first opened.
- Tasks: a project's finished task can be rerun (its changes are planned again) while no task runs; other
  projects' tasks cannot. Projects cannot be switched while a task runs (`task.busy`).
- Custom title bar: the header is the drag region (`data-tauri-drag-region`); minimize,
  maximize/restore and close buttons at the far right, JetBrains Windows style.
- No project open: the Packages view shows "Open a project" with an Open… button (native file dialog,
  `.uproject` filter) and the recent projects; the ⋮ menu has Open Project…, recent projects, Show in
  Explorer, Open in Unreal Editor.
- Pending changes live in app state (one per package); Apply… → `plan` → Apply Changes dialog →
  `apply` → Tasks view; the snapshot refreshes when the task ends.
- Mock-only URL parameters for screenshots: `lang=en|zh-CN`, `theme=dark|light`,
  `view=packages|graph|problems|tasks`, `dialog=recipes|plan|settings`,
  `section=<settings section>`, `select=<package>`, `noproject=1`.
- Scripts: `npm run dev`, `npm run build` (tsc + vite, output `app/dist`), `npm test` (vitest:
  i18n parity — every key in both files, every `msg.` code from §8 present — and formatting helpers).

## 12. Tauri shell (`src-tauri/`)

Tauri 2. `productName` "uBot Manager", `identifier` `com.astreox.ubotmanager`, exe `ubot-manager.exe`
(a GUI-subsystem exe in release builds, so no console window), window 1280×800 (min 960×600),
`decorations: false`, centered. `frontendDist` `../app/dist`, `devUrl` `http://localhost:5173`. Plugins:
dialog, opener, single-instance. The capability (`capabilities/default.json`) allows the app commands,
the window controls (minimize, maximize, close, dragging), listening to events, the native open dialog,
opening web links, revealing a path in Explorer, and opening a path only when it is a `*.uproject`
("Open in Unreal Editor"). Bundling targets NSIS per-user (`installMode: currentUser`), but verification
builds use `--no-bundle`. Only `npm run tauri build` (the Tauri CLI) enables the `custom-protocol`
feature that embeds `app/dist`; a plain `cargo build` yields a development build that loads `devUrl`.
`gen/schemas/` is generated by every build and is not committed (`src-tauri/.gitignore`). Icons generated
with `npx tauri icon` from `src-tauri/icons/source.png` (orange rounded square with white "uB", as in the
header logo).

Command line: `ubot-manager.exe [--project <path>]` (also `--project=<path>`; the last one wins, other
arguments are ignored). Without it the shell opens `lastProject`. An invalid `--project` shows an error
dialog and leaves the last project open.

Startup, in a background thread so the window appears at once:

1. unless `UBOT_MANAGER_HOME` is set (test and smoke runs), write the exe's own path to
   `HKCU\Software\AstreoX\uBot Manager` value `Executable` (REG_SZ; skipped when it is already there),
   which is how the UE panel finds the manager (§9.3);
2. open the project from `--project` (else `lastProject`) and set the window title to
   `<Project> - uBot Manager` (visible in the taskbar; the custom title bar shows no project name);
3. **quick scan**: `rescan()` from local files and caches, no network (sources never cached show "not
   fetched yet"), published as `ubot://snapshot`;
4. **full refresh**: `refresh(sync = settings.index.syncOnStart)`, published as `ubot://snapshot`.

The frontend asks for its snapshot as soon as it loads and, when `syncOnStart` is set, runs its own
`refresh(sync = true)`. The shell makes those two calls cheap: `get_snapshot` waits for the quick scan
instead of scanning again, and the first `refresh(sync = true)` that arrives while the startup sync runs,
or within 30 s after it, is answered by that sync, so the index is fetched once.

A second `ubot-manager.exe` exits at once after handing its `--project` (made absolute against its working
directory) to the first instance, which opens it, publishes `ubot://snapshot`, and un-minimizes and
focuses its window; an error is shown in a dialog. Closing the window while a task runs cancels the task
and waits up to 10 s for it to stop its processes and remove a partial clone.

## 13. Verification

- Rust: `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`. Unit tests keep their
  fixtures in temporary folders under `%TEMP%` (the `tempfile` crate) and pass explicit `AppDirs`; tests
  that start the CLI set `UBOT_MANAGER_HOME` and `UBOT_MANAGER_PERMANENT_DELETE=1`. `src-tauri` has a
  contract test: the model's JSON shapes are written to `app/src/api/contract.fixture.ts`, which
  `npm test` type-checks against `types.ts`; after changing the model run
  `UBOT_UPDATE_CONTRACT=1 cargo test -p ubot-manager contract`, then `npm test` in `app/`.
- Index: `node scripts/sync-index.mjs --check` fails when the embedded copy differs from uBotCore's
  `Index/index.json`; `node scripts/sync-index.mjs` refreshes it. `real_index.rs` validates both copies
  and checks them against `index.schema.json` (`UBOTCORE_DIR` or the workspace layout; skipped without
  uBotCore).
- CLI end-to-end (temp dirs only; `UBOT_PLUGINS_DIR` or the workspace layout, skipped without the
  repositories or git): local bare repos with tags built from the real uBotCore / uBotSensor / uBotROS
  working trees, a local index pointing at them, a temp copy of a content-only project;
  `ubot apply install:UBotROS --no-build --yes` clones 3 packages and enables them; update, disable
  (dependents), uninstall (permanent delete instead of the Recycle Bin), problems, `.uproject`
  formatting preserved.
- Frontend: `npm test`, `npm run build`, and screenshots of every view and dialog in both languages and
  both themes (`vite preview` with the mock URL parameters of §11), compared with the design.
- UE: build the host project (`Build.bat <Host>Editor Win64 Development -Project=<host .uproject>
  -WaitMutex -NoHotReloadFromIDE`), run `Automation RunTests UBot` headless, and check the panel
  (`uBot.OpenPanel`) in both editor languages.
- App: `npm run tauri build -- --no-bundle` (in `app/`) produces `target/release/ubot-manager.exe`;
  launch it with `--project` on a temp project copy and `UBOT_MANAGER_HOME` set, and confirm it starts,
  loads the snapshot and exits cleanly.
