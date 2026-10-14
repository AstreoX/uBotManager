<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/brand/ubot-mark-light.svg">
    <source media="(prefers-color-scheme: light)" srcset="docs/brand/ubot-mark-dark.svg">
    <img alt="uBot" src="docs/brand/ubot-mark-dark.svg" height="88">
  </picture>
</p>
<h1 align="center">uBot Manager</h1>
<p align="center"><b>在 Unreal Engine 5.5 工程里安装、更新、构建 uBot Package。</b></p>
<p align="center">
  <a href="https://github.com/AstreoX/uBotManager/releases"><img alt="release" src="https://img.shields.io/github/v/release/AstreoX/uBotManager?label=release"></a>
  <a href="LICENSE"><img alt="license" src="https://img.shields.io/github/license/AstreoX/uBotManager"></a>
  <img alt="Unreal Engine 5.5" src="https://img.shields.io/badge/Unreal%20Engine-5.5-313131">
</p>
<p align="center">
  <a href="https://astreox.github.io/uBotDocs/manager/">文档</a> ·
  <a href="https://github.com/AstreoX/uBotManager/releases">发布版本</a> ·
  <a href="https://github.com/users/AstreoX/projects/2">路线图</a> ·
  <a href="https://github.com/AstreoX/uBotManager/issues">问题反馈</a>
</p>

[English](README.md) | **简体中文**

## 简介

uBot Manager 是一个 Windows 桌面程序，在 Unreal Engine 5.5 工程里安装、更新、启用、禁用和卸载 uBot Package。它先处理依赖并给出计划，由你确认后，再下载预编译包，或克隆源码并编译。命令行 `ubot` 使用同一个核心，适合脚本和 CI。

- **先看计划，再执行。** 更改先进入待应用列表，以步骤列出；确认之前不会执行任何步骤。
- **优先预编译。** 有适合当前引擎的预编译包时，下载并校验后直接使用；否则克隆对应标签并编译。
- **依赖一并处理。** 需要的 Package 和引擎插件会一起安装或启用。
- **Recipe。** 一次应用一组现成的组合，例如 ROS 2 传感器。
- **问题和快速修复。** 依赖缺失或版本不符时列出问题，并给出可加入待应用列表的修复。
- **依赖图、任务日志、主题。** 依赖图，每个任务的日志，简体中文和英文界面，深色和浅色主题。

## 安装

1. 到 [Releases](https://github.com/AstreoX/uBotManager/releases) 下载 `uBot-Manager_<版本>_x64-setup.exe` 并运行。安装包按当前用户安装，不需要管理员权限，可以在向导里选择目录。
2. 安装包没有代码签名，Windows SmartScreen 可能提示「未知发布者」：点「更多信息 > 仍要运行」。

其他方式：

- **命令行 `ubot`。** 安装包里没有它，用 `cargo build --release -p ubot-cli` 构建（Rust stable，MSVC），生成 `target\release\ubot.exe`。
- **从源码构建。** 自己构建桌面程序，见 [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)。
- **不用 Manager。** 手动克隆插件，见[安装指南](https://astreox.github.io/uBotDocs/start/install/)。

运行要求：Windows 10 或 11（WebView2 运行库，Windows 11 已自带）和 Unreal Engine 5.5。从源码安装需要 git，编译需要 Visual Studio 2022（含 C++ 工具链）；只安装预编译包时两者都不需要。

## 快速开始

1. 启动 uBot Manager，点 `打开…`，选择你的 `.uproject`。
2. 点 `Recipe`，选 `ROS 2 传感器`，点 `查看更改…`，再点 `应用`。
3. 用 UE 5.5 打开工程：`Window > uBot` 里列出已加载的 Package。

在命令行里做同样的事，其中 `plan` 不会改动任何东西：

```powershell
ubot --project .\MyRobotSim.uproject status
ubot --project .\MyRobotSim.uproject plan install:UBotROS
ubot --project .\MyRobotSim.uproject apply install:UBotROS --yes
```

桌面程序和命令行共用一个核心：

```mermaid
flowchart LR
  App["桌面程序"] --> Core["核心（Rust）"]
  CLI["命令行 ubot"] --> Core
  Core -->|读取| Index["Package 索引"]
  Core -->|安装| Plugins["Plugins/uBot：预编译包，或 git 克隆 + 编译"]
  Core -->|修改| Proj[".uproject"]
```

## 文档

| 我想 ... | 去这里 |
| --- | --- |
| 把 uBot Package 装进工程 | [安装 uBot](https://astreox.github.io/uBotDocs/start/install/) |
| 了解视图、任务和问题 | [界面导览](https://astreox.github.io/uBotDocs/manager/) |
| 安装、更新或移除 Package | [安装、更新、移除插件](https://astreox.github.io/uBotDocs/manager/packages/) |
| 一次装好一组 Package | [组合方案](https://astreox.github.io/uBotDocs/manager/recipes/) |
| 修改设置和索引源 | [uBot Manager 设置](https://astreox.github.io/uBotDocs/manager/settings/) |
| 在脚本或 CI 里使用 | [命令行 `ubot`](https://astreox.github.io/uBotDocs/manager/cli/) |
| 编写自己的 uBot Package | [编写自己的 uBot Package](https://astreox.github.io/uBotDocs/develop/package-authoring/) |
| 构建或发布 Manager | [DEVELOPMENT](docs/DEVELOPMENT.md) · [RELEASING](docs/RELEASING.md) · [INTERNALS](docs/INTERNALS.md) |

## uBot 系列

| 仓库 | 说明 |
| --- | --- |
| [uBotCore](https://github.com/AstreoX/uBotCore) | 仿真时钟、帧头、共享接口、插件管理 |
| [uBotSensor](https://github.com/AstreoX/uBotSensor) | 位姿、IMU、里程计、2D/3D 激光雷达、RGB 与深度相机 |
| [uBotROS](https://github.com/AstreoX/uBotROS) | 通过 TCP 桥接入 ROS 2：传感器、TF、/clock、cmd_vel |
| [uBotAerial](https://github.com/AstreoX/uBotAerial) | 多旋翼无人机：简化飞行、刚体动力学、风 |
| **[uBotManager](https://github.com/AstreoX/uBotManager)** | **在 UE 工程里安装、更新、构建 uBot 包的桌面程序和命令行** |
| [uBotDocs](https://github.com/AstreoX/uBotDocs) | 文档站：https://astreox.github.io/uBotDocs/ |

## 开发

- 需要 Rust（stable，MSVC）和 Node.js 22.12 或更新版本。在 `app/` 里运行 `npm ci` 和 `npm run tauri build -- --no-bundle`；在仓库根目录运行 `cargo build --release -p ubot-cli`。
- 测试：`cargo test --workspace`，以及在 `app/` 里运行 `npm test`。
- 细节见 [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)，发布流程见 [docs/RELEASING.md](docs/RELEASING.md)。
- 问题和建议请提到 [Issues](https://github.com/AstreoX/uBotManager/issues)。

## 许可

Copyright 2026 AstreoX。本项目以 [Apache License 2.0](LICENSE) 发布，可以自由使用、修改和再分发（包括商业用途），再分发时需保留 [LICENSE](LICENSE) 与 [NOTICE](NOTICE) 文件，并注明你修改过的文件。

### Code signing policy

发布的安装包由 [`Release` 工作流](.github/workflows/release.yml)从本仓库的源码构建后签名。签名正在向 SignPath Foundation 申请，生效之前发布的安装包没有签名。

Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by [SignPath Foundation](https://signpath.org/).

- Committers and reviewers: [AstreoX](https://github.com/AstreoX)
- Approvers: [AstreoX](https://github.com/AstreoX)

隐私说明：uBot Manager 不收集、不发送任何个人信息或使用数据。它只在两种情况下联网：同步包索引（默认启动时同步，可以在 `设置 > Package 索引 > 启动时同步索引` 关闭，也可以点刷新按钮手动同步）；安装或更新 Package 时，从索引里写的地址下载预编译包或克隆仓库（默认都在 GitHub）。向 GitHub 发出的请求适用 [GitHub General Privacy Statement](https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement)。
