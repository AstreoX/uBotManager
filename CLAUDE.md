# CLAUDE.md

uBot Manager installs, updates, enables, disables and removes uBot Unreal Engine 5.5 plugins ("packages") in a `.uproject`, reading the uBot package index (format 2). It is a Tauri 2 desktop app (Rust core, React frontend) plus a CLI, `ubot`, built on the same core. A thin uBot panel in Unreal Editor only shows status and launches the app. Engine discovery, C++ builds and the installer are Windows-only.

## Layout

| Path | Owns |
|---|---|
| `crates/ubot-core` | Library: index and sources (`index.rs`, `sources.rs`), versions (`version.rs`), project and `.uproject` model (`project.rs`, `uproject.rs`), problems, planning (`plan.rs`), task execution (`exec.rs`), prebuilt archives (`prebuilt.rs`, `download.rs`), settings, message codes (`msg.rs`). `assets/index.json` is the embedded index copy |
| `crates/ubot-cli` | The `ubot` binary (clap): `main.rs`, `package.rs` (`ubot package`), `render.rs`; tests in `tests/` |
| `src-tauri` | Tauri shell, crate `ubot-manager`: commands, events, single instance, startup, registry entry; `tauri.conf.json`, `capabilities/default.json` |
| `app` | Vite + React + TypeScript frontend: `src/api` (`types.ts`, Tauri and mock backends), `src/views`, `src/dialogs`, `src/i18n`, `src/lib`, `src/theme.css`; `scripts/tauri.mjs` points the Tauri CLI at `src-tauri` |
| `scripts/sync-index.mjs` | Refreshes the embedded index copy |
| `docs/SPEC.md`, `docs/brand/` | The contract; logo assets |

The Unreal Editor panel is not in this repo; it lives in the uBotCore plugin repository.

## docs/SPEC.md is the contract

- `docs/SPEC.md` binds core, CLI, shell, frontend and the UE side. Read the section for your area before editing; if code and SPEC disagree, fix one of them.
- A change to any interface it describes must update SPEC.md in the same commit: index format (§1), versions (§2), project model (§3), problems (§4), planning and tasks (§5), settings (§6), commands, events and types (§7), message codes (§8), UE-side files (§9), CLI (§10), frontend (§11), shell (§12).
- Files shared with the UE panel (SPEC §9): `Saved/uBot/index.json` written by `manager.rs`, `Saved/uBot/status.json` read by `status.rs`, registry value `Executable` under `HKCU\Software\AstreoX\uBot Manager` written by `src-tauri/src/registry.rs`, and the `--project` argument (`src-tauri/src/args.rs`). Changing one also needs a uBotCore change.
- Rust mirrors of uBotCore C++: `version.rs` (`UBotSemVer`), `index::is_safe_ref`, the URL policy in `url.rs`. Keep grammar and test vectors identical.

## Commands

Toolchain: stable Rust 1.88 or newer, Node.js 22.12 or 24 with npm, git. Rust commands run from the repo root:

```
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
cargo test -p ubot-core -p ubot-cli      # without the Tauri shell
```

Frontend commands run in `app/`:

```
npm ci
npm run dev                  # Vite on :5173 with the in-memory mock backend, no Tauri
npm run tauri dev            # desktop app with hot reload
npm test                     # vitest run, src/**/*.test.ts
npm run build                # tsc --noEmit + vite build into app/dist
npm run tauri build -- --no-bundle   # target/release/ubot-manager.exe
```

- No ESLint or Prettier is configured; `npm run build` is the type check, `cargo clippy` and `cargo fmt` are the Rust lint.
- `cargo build -p ubot-manager` alone yields a dev build that loads `http://localhost:5173`. Use `npm run tauri build` for a real build. `cargo build --release -p ubot-cli` builds `ubot`.
- Mock-only URL parameters for `npm run dev`: `lang`, `theme`, `view`, `dialog`, `section`, `select`, `noproject=1`, `prebuilt=0`.
- After changing the model (`crates/ubot-core/src/model.rs`) or `app/src/api/types.ts`, set `UBOT_UPDATE_CONTRACT=1`, run `cargo test -p ubot-manager contract` to regenerate `app/src/api/contract.fixture.ts` (generated, do not hand-edit), then run `npm test`.
- `node scripts/sync-index.mjs` refreshes the embedded index from uBotCore's `Index/index.json` (`--from <file>` or `UBOTCORE_INDEX` to choose the source; `--check` only compares).
- Set `UBOT_MANAGER_HOME` to a temp directory for any run of the app or CLI that is not a unit test. It moves settings, history and caches and skips the registry write.

CLI (`ubot [--project <.uproject>] [--index <file|url>]... [--offline] [--json] <command>`):

| Command | What it does |
|---|---|
| `status`, `info <package>`, `problems` | Show packages, one package, problems |
| `plan <change>...` | Show the plan; exit 1 on errors. Change = `kind:Name[@ver]` with kind `install`, `update`, `enable`, `disable`, `uninstall`; `update` needs a version |
| `apply <change>... [--no-build] [--run-tests] [--reopen-editor] [--yes]` | Plan and run |
| `recipes`, `recipe <id> [--no-build] [--yes]` | List recipes; install missing and enable disabled packages of one |
| `engines` | Installed Unreal Engine versions |
| `package <Name[@ver]>... --out <dir> [--engine <id\|root>] [--url-template <tpl>] [--update-index <file>] [--with-symbols] [--keep-work]` | Build from source with a launcher engine in `<out>/_work`, pack `<Name>-<version>-UE<x.y>-Win64.zip` plus `.sha256`, optionally write the `binaries` entries into an index file |
| `index validate <file>`, `index show` | Validate an index (exit 1 on errors); show the merged index |

### Cloud sessions (Linux, no Unreal Engine)

- Usually runnable: `cargo test -p ubot-core -p ubot-cli`, `cargo fmt --all -- --check`, and in `app/` `npm ci`, `npm test`, `npm run build`.
- `src-tauri` links the platform webview; on Linux it needs Tauri's WebKitGTK system packages, so `--workspace` commands may not build without them.
- Not runnable on Linux: tests gated `#[cfg(windows)]` (the opener scope test in `src-tauri/src/capabilities.rs`, the registry test, all of `crates/ubot-cli/tests/package.rs`), anything that needs UE or UBT, and the NSIS installer. Say so in the PR instead of claiming they passed.
- Tests that need sibling uBot repositories (`UBOT_PLUGINS_DIR`, `UBOTCORE_DIR`) or git skip themselves when absent.

## Package index

- Format 2 only: `FORMAT_VERSION` in `crates/ubot-core/src/index.rs`; `index::parse` rejects any other `formatVersion`; `index::validate_text` backs `ubot index validate` (SPEC §1).
- Default remote URL: `REMOTE_INDEX_URL` in `crates/ubot-core/src/settings.rs`. Fetch, cache and merge are in `sources.rs`; with no usable source the embedded `assets/index.json` is used. Do not hand-edit that copy; run the sync script.
- Versions and constraints use the grammar in `version.rs`. Do not use the `semver` crate.
- `planned: true` versions are announced, not installable; a package with no non-planned version is planned (`IndexPackage::is_planned`, `plan.planned`). The UI lists planned packages only while `index.showPlanned` is on.
- `requires` is resolved recursively by `plan.rs`; unmet or cyclic requirements surface as problems (`problems.rs`).
- `binaries` is `binaries["<Major.Minor>"]["Win64"] = { url, sha256, size? }`, parsed by `index::parse_binary`. An entry is used only if its URL, SHA-256 and size are valid (`IndexVersion::prebuilt`), the engine is a launcher install and `install.usePrebuilt` is on; otherwise the package is cloned and built. Only `Win64` is read (`prebuilt::PLATFORM`).

## Security rules

- Opener scope: `src-tauri/capabilities/default.json` allows `opener:allow-open-path` only for `**/*.uproject`. The test `opener_may_open_projects_only` in `src-tauri/src/capabilities.rs` (Windows only) pins it. Never widen it; give any new permission the narrowest scope. The CSP is in `tauri.conf.json`.
- Downloads (`download.rs`, `prebuilt.rs`): the index SHA-256 is verified before extraction, `size` is enforced when listed, 2 GiB cap, HTTPS only (plain http only to the local machine), at most 5 redirects, never https to http. Archive entries are checked before anything is written (`extract_package`). `.ubot-install.json` is written by the manager, never taken from an archive.
- Every URL from an index passes `url.rs` (`check_repository_url`, `check_binary_url`). Credentials are redacted (`redact_credentials`, `strip_url_secrets`) in logs, errors, UI, install markers and `ubot index show`. Never log a full download URL.
- An installed package's `.git/config` is untrusted (`git.rs`): git runs with `GIT_TERMINAL_PROMPT=0` and `core.fsmonitor=false`, and checkouts whose config names programs are refused.
- Writes: settings under `%APPDATA%\AstreoX\uBot Manager`, history and cache under `%LOCALAPPDATA%\AstreoX\uBot Manager` (`paths.rs`; `UBOT_MANAGER_HOME` moves all). In a project: the `.uproject`, `<install.directory>/<folder>` (default `Plugins/uBot`), `Saved/uBot/index.json`, `Config/DefaultUBotROS.ini`. Write files with `paths::write_atomic`; `.uproject` edits keep formatting (`uproject.rs`).
- Uninstall moves the folder to the Recycle Bin and refuses folders outside the project. `UBOT_MANAGER_PERMANENT_DELETE=1` is a CLI-only switch for tests.
- No real tokens or credentials in source, tests or logs.

## UI

- Languages: `en` and `zh-CN`, in `app/src/i18n/en.json` and `app/src/i18n/zh-CN.json`. Files are two-level (`{ "section": { "rest.of.key": "text" } }`); use `t`, `tn` (`.one`/`.other`) and `tm` from `app/src/i18n/index.ts`.
- Every user-visible string goes in both files with the same keys and `{placeholders}`; do not hardcode text in TSX. `i18n.test.ts` fails on key or placeholder mismatch, empty strings, missing backend codes, zh-CN typography (spaces between CJK and Latin or digits, full-width punctuation) and the character 包 ("Package" stays English).
- A new backend message code needs the `EN` table in `crates/ubot-core/src/msg.rs`, both locale files under `msg.`, SPEC §8 and the `SPEC_CODES` list in `i18n.test.ts`. CLI output is English.
- Style (SPEC §11): plain CSS with tokens in `app/src/theme.css` (dark default, light, system), no UI framework, JetBrains-IDE look, no pill shapes, fonts bundled locally, buttons that open dialogs end with "…".
- `app/src/api/mock.ts` stands in for the backend outside Tauri; update it when a command or type changes.

## Conventions

- Rust: edition 2021, default rustfmt, clippy clean with `-D warnings`. Module headers (`//!`) cite the SPEC section. Keep `#[cfg(windows)]` code paired with a non-Windows fallback so crates compile on Linux.
- Rust tests: unit tests in `#[cfg(test)] mod tests` beside the code, integration tests in `crates/*/tests/`. Use `tempfile` directories; tests that spawn the CLI set `UBOT_MANAGER_HOME` and `UBOT_MANAGER_PERMANENT_DELETE=1`.
- TypeScript: `strict`, two spaces, double quotes, semicolons, camelCase; `snake_case` Tauri command names. Tests are `src/**/*.test.ts` beside the code and run in vitest's node environment (no DOM), so keep testable logic in `src/lib` and `src/api`.
- Add a focused test with each behavior change or fix.
- Commits: Conventional Commits, imperative mood, optional scope naming the part: `feat(cli): ...`, `fix(core): ...`, `docs: ...`. Types in history: `feat`, `build`, `chore`, `docs`.

## Release

- Keep these versions equal: `[workspace.package] version` in root `Cargo.toml` (all three crates inherit it), `version` in `src-tauri/tauri.conf.json`, `version` in `app/package.json` (and the matching top entry of `app/package-lock.json`).
- Tag the release commit `vX.Y.Z`.
- The NSIS installer (per-user, `bundle.targets` in `tauri.conf.json`) is built on Windows only: in `app/`, `npm ci` then `npm run tauri build`; output is under `target/release/bundle/nsis/`.

## Never commit

`target/`, `app/node_modules/`, `app/dist/`, `src-tauri/gen/schemas`, `*.log`, `.vs/`, `.idea/` (see the `.gitignore` files).
