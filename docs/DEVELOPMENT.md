# 开发

> 从 README 迁出，文字未改动：从源码构建和仓库结构。只有一处链接改了：指向 README 小节「发布 uBot Manager」的锚点，改成了 `RELEASING.md` 里的同名小节。

## 从源码构建

需要：

- Rust stable，MSVC 工具链。代码用到 `slice::as_chunks`，需要 Rust 1.88 或更新，开发使用 1.99。Rust 可以装在任何位置，保证 `cargo` 在 `PATH` 里即可。
- Node.js 22.12 或 24 以上（开发使用 24），带 npm。
- Visual Studio 2022 的 C++ 生成工具。

```powershell
cd app
npm ci
npm run tauri build -- --no-bundle     # 生成 target\release\ubot-manager.exe
cd ..
cargo build --release -p ubot-cli      # 生成 target\release\ubot.exe
```

- `npm run tauri build` 由 Tauri CLI 驱动：先运行 `npm run check-index` 检查内置索引副本是否最新，不一致就失败（在仓库根目录运行 `node scripts/sync-index.mjs` 更新）；通过后运行 `npm run build` 生成 `app/dist`，再编译外壳并把前端嵌入。不要直接用 `cargo build` 构建 `ubot-manager`，那样得到的是开发构建，会去加载 `http://localhost:5173`。
- 未用 `--from` 或 `UBOTCORE_INDEX` 指定源、且工作区里的 uBotCore 索引读不到时，构建检查会从 GitHub 上的 uBotCore 比较。离线构建可设 `UBOT_SKIP_INDEX_CHECK=1` 跳过检查。
- 去掉 `--no-bundle` 会按 `tauri.conf.json` 生成 NSIS 安装包 `target\release\bundle\nsis\uBot Manager_<版本>_x64-setup.exe`：按当前用户安装，可在安装向导里选择目录，界面随系统语言显示中文或英文。首次打包时 Tauri 会把 NSIS 下载到 `%LOCALAPPDATA%\tauri`（不能改位置；想放到别的盘，可以先把这个目录建成指向别处的目录联接）。本地打出的安装包没有代码签名，SmartScreen 会提示未知发布者；发布用的安装包由 GitHub Actions 构建，见 [发布 uBot Manager](RELEASING.md#发布-ubot-manager)。
- 开发：在 `app/` 里运行 `npm run tauri dev` 启动带热更新的桌面程序；`npm run dev` 只启动前端，页面使用内存里的 mock 后端，可以用参数直达某个界面，例如 `http://localhost:5173/?lang=zh-CN&theme=light&view=problems`，参数有 `lang`、`theme`、`view`、`dialog`、`section`、`select`、`noproject`，以及 `prebuilt=0`（“有预编译版本时优先使用”从关闭开始，安装就走克隆加编译；默认是开）。

测试：

```powershell
cargo test --workspace
cargo clippy --workspace -- -D warnings
cd app
npm test                  # vitest
npm run build             # tsc 类型检查 + vite 构建
```

- 集成测试需要真实仓库时，在 uBot 工作区布局下自动找到 `Plugins/uBot` 里的 uBotCore、uBotSensor、uBotROS，也可以用 `UBOT_PLUGINS_DIR` 和 `UBOTCORE_DIR` 指定；找不到或没有 git 就跳过。
- 改动 `crates/ubot-core/src/model.rs` 里的数据结构后，设置环境变量 `UBOT_UPDATE_CONTRACT=1` 再运行 `cargo test -p ubot-manager contract`，重新生成 `app/src/api/contract.fixture.ts`；然后运行 `npm test`，它会检查前端的 `types.ts` 与后端是否一致。
- 更新内置的索引副本：`node scripts/sync-index.mjs`（`--check` 只比较不写入）。默认从工作区里的 `Plugins/uBot/uBotCore/Index/index.json` 复制，也可以用 `--from` 或环境变量 `UBOTCORE_INDEX` 指定。`--check` 在未指定源且工作区索引读不到时，会回退到 GitHub 上的 uBotCore 索引进行比较。
- 手动运行时设置 `UBOT_MANAGER_HOME` 指向一个临时目录，避免动到自己的设置。

## 仓库结构

| 路径 | 内容 |
| --- | --- |
| `crates/ubot-core` | 核心库：索引、项目模型、版本约束、问题、计划、任务执行、设置，以及预编译包的下载、校验、解压和打包。`assets/index.json` 是内置的索引副本 |
| `crates/ubot-cli` | 命令行 `ubot`，包括 `ubot package` |
| `src-tauri` | Tauri 外壳：命令、事件、单实例、启动流程、注册表登记 |
| `app` | 前端（Vite、React、TypeScript）：视图、对话框、中英文语言包、mock 后端 |
| `scripts` | `sync-index.mjs` |
| `.github/workflows` | `release.yml`：构建、签名并起草 Release |
| `docs/SPEC.md` | 实现所依据的技术规格，英文 |
