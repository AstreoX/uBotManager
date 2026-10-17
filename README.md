<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/brand/ubot-mark-light.svg">
    <source media="(prefers-color-scheme: light)" srcset="docs/brand/ubot-mark-dark.svg">
    <img alt="uBot" src="docs/brand/ubot-mark-dark.svg" height="88">
  </picture>
</p>
<h1 align="center">uBot Manager</h1>
<p align="center"><b>Install, update and build uBot packages in your Unreal Engine 5.5 projects.</b></p>
<p align="center">
  <a href="https://github.com/AstreoX/uBotManager/releases"><img alt="release" src="https://img.shields.io/github/v/release/AstreoX/uBotManager?label=release"></a>
  <a href="LICENSE"><img alt="license" src="https://img.shields.io/github/license/AstreoX/uBotManager"></a>
  <img alt="Unreal Engine 5.5" src="https://img.shields.io/badge/Unreal%20Engine-5.5-313131">
</p>
<p align="center">
  <a href="https://astreox.github.io/uBotDocs/manager/">Documentation</a> ·
  <a href="https://github.com/AstreoX/uBotManager/releases">Releases</a> ·
  <a href="https://github.com/users/AstreoX/projects/2">Roadmap</a> ·
  <a href="https://github.com/AstreoX/uBotManager/issues">Issues</a>
</p>

**English** | [简体中文](README.zh-CN.md)

## What it is

uBot Manager is a Windows desktop app that installs, updates, enables, disables and removes uBot packages in an Unreal Engine 5.5 project. It resolves dependencies, shows the plan, then downloads a prebuilt package or clones and builds one from source. The `ubot` command line runs the same core for scripts and CI.

- **Review before applying.** Every change is listed as steps and runs only after you confirm.
- **Prebuilt or source.** Installs a prebuilt package when the index has one for your launcher-installed engine, and builds from source otherwise.
- **Dependencies included.** Required packages and engine plugins are installed or enabled together.
- **Recipes.** Install a preset set of packages, such as ROS 2 Sensors, in one step.
- **Problems and quick fixes.** Dependency problems are listed with quick fixes.
- **Graph, task log, themes.** A dependency graph, a log per task, English and Simplified Chinese, dark and light.

## Install

1. Download `uBot-Manager_<version>_x64-setup.exe` from [Releases](https://github.com/AstreoX/uBotManager/releases) and run it. It installs for the current user without administrator rights; the install folder is selectable.
2. The installer is not code-signed. SmartScreen reports an unknown publisher; choose *More info > Run anyway*.

Alternatives:

- **`ubot` command line.** Not included in the installer. Build it with `cargo build --release -p ubot-cli` (Rust stable, MSVC); the output is `target\release\ubot.exe`.
- **From source.** Build the desktop app, see [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).
- **Without the Manager.** Install the plugins manually, see the [install guide](https://astreox.github.io/uBotDocs/start/install/).

Requires Windows 10 or 11 (WebView2 runtime, included in Windows 11) and Unreal Engine 5.5. Installing from source also requires Git and Visual Studio 2022 with the C++ toolchain.

## Quick start

1. Start uBot Manager, click `Open…` and select your `.uproject`.
2. Click `Recipes`, select `ROS 2 Sensors`, then click `Review Changes…` and `Apply`.
3. Open the project in UE 5.5. `Window > uBot` lists the loaded packages.

On the command line (`plan` only prints the plan):

```powershell
ubot --project .\MyRobotSim.uproject status
ubot --project .\MyRobotSim.uproject plan install:UBotROS
ubot --project .\MyRobotSim.uproject apply install:UBotROS --yes
```

The desktop app and the command line share one core:

```mermaid
flowchart LR
  App["Desktop app"] --> Core["Core (Rust)"]
  CLI["ubot CLI"] --> Core
  Core -->|reads| Index["Package index"]
  Core -->|installs| Plugins["Plugins/uBot: prebuilt zip, or git clone + build"]
  Core -->|edits| Proj[".uproject"]
```

## Documentation

The documentation site and the files in `docs/` are in Simplified Chinese.

| I want to ... | Go to |
| --- | --- |
| Install uBot packages in a project | [Install uBot](https://astreox.github.io/uBotDocs/start/install/) |
| Learn the views, tasks and problems | [Interface tour](https://astreox.github.io/uBotDocs/manager/) |
| Install, update or remove packages | [Install, update, remove](https://astreox.github.io/uBotDocs/manager/packages/) |
| Install a set of packages at once | [Recipes](https://astreox.github.io/uBotDocs/manager/recipes/) |
| Change settings and index sources | [Settings](https://astreox.github.io/uBotDocs/manager/settings/) |
| Use it in scripts or CI | [`ubot` command line](https://astreox.github.io/uBotDocs/manager/cli/) |
| Write a uBot package | [Package authoring](https://astreox.github.io/uBotDocs/develop/package-authoring/) |
| Build or release the Manager | [DEVELOPMENT](docs/DEVELOPMENT.md) · [RELEASING](docs/RELEASING.md) · [INTERNALS](docs/INTERNALS.md) |

## uBot series

| Repository | What it is |
| --- | --- |
| [uBotCore](https://github.com/AstreoX/uBotCore) | Simulation clock, frame header, shared interfaces, package management |
| [uBotSensor](https://github.com/AstreoX/uBotSensor) | Pose, IMU, odometry, 2D/3D lidar, RGB and depth cameras |
| [uBotROS](https://github.com/AstreoX/uBotROS) | ROS 2 over a TCP bridge: sensors, TF, /clock, cmd_vel |
| [uBotAerial](https://github.com/AstreoX/uBotAerial) | Multirotor drones: simplified flight, rigid-body dynamics, wind |
| **[uBotManager](https://github.com/AstreoX/uBotManager)** | **Desktop app and CLI to install, update and build uBot packages** |
| [uBotDocs](https://github.com/AstreoX/uBotDocs) | Documentation site: https://astreox.github.io/uBotDocs/ |

## Development

- Needs Rust (stable, MSVC) and Node.js 22.12 or later. In `app/` run `npm ci` and `npm run tauri build -- --no-bundle`; at the root run `cargo build --release -p ubot-cli`.
- Test with `cargo test --workspace` and, in `app/`, `npm test`.
- Details are in [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md); the release process is in [docs/RELEASING.md](docs/RELEASING.md).
- Report bugs and ideas in [Issues](https://github.com/AstreoX/uBotManager/issues).

## License

Copyright 2026 AstreoX. Licensed under the [Apache License 2.0](LICENSE); see also [NOTICE](NOTICE).

### Code signing policy

Release installers are built from this repository's source by the [`Release` workflow](.github/workflows/release.yml) and then signed. Signing is being applied for with the SignPath Foundation; installers released before it takes effect are not signed.

Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by [SignPath Foundation](https://signpath.org/).

- Committers and reviewers: [AstreoX](https://github.com/AstreoX)
- Approvers: [AstreoX](https://github.com/AstreoX)

Privacy policy: uBot Manager does not collect or send personal data, usage data or telemetry. It connects to other systems only to:

- download the package index, by default from `raw.githubusercontent.com`: at startup (can be turned off with *Settings > Package Index > Sync index on startup*) and when you click the refresh button;
- download prebuilt packages or clone package repositories from the addresses listed in the index (by default `github.com`) when you install or update packages;
- ask the GitHub Releases API (`api.github.com`) for the latest uBot Manager release: once at startup (can be turned off with *Settings > About > Check for a new version of uBot Manager on startup*) and when you click *Check for Updates*. The request carries the app name and version as its user agent and nothing else; nothing is downloaded or installed.

Requests to GitHub are covered by the [GitHub General Privacy Statement](https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement).
