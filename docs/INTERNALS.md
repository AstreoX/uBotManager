# 补充细节

> 从 README 迁出，文字未改动。文档站（<https://astreox.github.io/uBotDocs/manager/>）没有覆盖这些细节。只有一处改动：原 README 第 199 行里指向 README 小节的锚点，改成了文档站的页面链接。

## 简介

- 编译、运行测试和重新打开编辑器调用的是 UBT 的 `Build.bat` 和引擎里的 Windows 可执行文件，引擎位置也来自 Windows 注册表，所以目前只支持 Windows。

## 启动

- `ubot-manager.exe` 可以放在任意位置。编辑器找不到它时会弹出通知，指向 <https://github.com/AstreoX/uBotManager/releases>。

### 引擎

uBot Manager 按 `.uproject` 的 `EngineAssociation` 找引擎：

| `EngineAssociation` | 查找位置 |
| --- | --- |
| `5.5` 这类版本号 | `C:\ProgramData\Epic\UnrealEngineLauncher\LauncherInstalled.dat`，其次是注册表 `HKLM\SOFTWARE\EpicGames\Unreal Engine\<版本>` |
| `{GUID}` | 注册表 `HKCU\Software\Epic Games\Unreal Engine\Builds` 中以该 GUID 命名的值（源码构建） |
| 空 | 从工程目录向上找 `Engine/Build/Build.version` |

`设置 > 编译 > 引擎` 可以改用另一个已注册的引擎；`settings.json` 里的 `build.engineId` 也可以直接写引擎根目录。引擎版本取自 `Engine/Build/Build.version`。找不到引擎时，问题列表会报告；需要编译的更改无法应用，计划会报错“找不到此项目的 UE 引擎”。

编译目标一般是 `<项目名>Editor`。纯内容工程、且插件都在 `<项目>/Plugins` 下时，这个目标可能不存在，改为编译引擎自带的 `UnrealEditor`（带 `-Project=`）。工程有 `Source/*.Target.cs`，或有插件放在 `AdditionalPluginDirectories` 里时用 `<项目名>Editor`。`<项目名>Editor` 编译时如果 UBT 报找不到目标规则文件，会自动改用 `UnrealEditor` 重试一次。

## Package 索引

### 失败与回退

- 远程索引下载成功后缓存到 `%LOCALAPPDATA%\AstreoX\uBot Manager\cache\index-ubot.json`，下载失败时使用缓存；失败原因可以在标题栏刷新按钮的提示里看到。
- 没有缓存时，靠后的索引源（一般是本地副本）顶上；所有索引源都没有给出索引时，使用编译进程序的内置副本（由 `scripts/sync-index.mjs` 从 uBotCore 的 `Index/index.json` 复制）。

### 同步时机

每次刷新后，uBot Manager 把合并后的索引写入 `<项目>/Saved/uBot/index.json`，编辑器里的 uBot 面板读取它；内容没有变化时不重写。

## 预编译安装

### 何时使用

2. 项目的引擎是 Epic 启动器安装的，即 `EngineAssociation` 是 `5.5` 这类版本号。启动器版本的二进制在每台机器上都一样；源码构建的引擎，以及 `settings.json` 的 `build.engineId` 里直接写成引擎根目录的引擎，不使用预编译包。

### 下载与解压

计划里的 `下载` 步骤（日志里逐行可见）依次做这些事：

1. 从索引里的地址下载到安装目录里的临时文件 `.ubot-download-<名称>.zip.part`。`file://` 地址和本地绝对路径（团队镜像）直接复制。HTTPS 下载遵循 `HTTPS_PROXY` 和 `HTTP_PROXY`，连接和读取各 30 秒超时，最多跟随 5 次重定向，不会从 https 退到 http。大小上限 2 GiB；索引写了 `size` 时，必须正好相等。步骤的进度显示已下载的字节数，如 `12.3 / 45.6 MB`。
2. 校验 SHA-256，和索引里的不一致就失败，并删除下载的文件。
3. 解压到 `.ubot-extract-<名称>`，拒绝绝对路径、盘符、`..`、符号链接和 `.git` 目录，也拒绝顶层不是唯一一个、以 Package 命名的文件夹的压缩包。
4. 检查 `<名称>.uplugin` 的 `VersionName` 等于要安装的版本；有 C++ 模块的插件还必须带着 `Binaries/Win64` 里的 `UnrealEditor-*.dll` 和 `UnrealEditor.modules`。
5. 写入安装标记 `.ubot-install.json`（见下）。标记由 uBot Manager 写，不取自压缩包。
6. 已有同名文件夹（更新、重新安装）时，先把它挪开，新文件夹就位后再把旧的移到回收站。

失败或取消时，临时文件和文件夹都会删除；已经安装的文件夹在第 6 步之前不会被改动。安装目录里以 `.ubot-` 开头的文件夹是 uBot Manager 自己的，扫描项目时会被跳过。

安装标记 `.ubot-install.json` 记录这个文件夹是怎么来的：

```json
{
    "kind": "prebuilt",
    "package": "UBotSensor",
    "version": "0.1.0",
    "engine": "5.5",
    "platform": "Win64",
    "url": "https://github.com/AstreoX/uBotSensor/releases/download/v0.1.0/UBotSensor-0.1.0-UE5.5-Win64.zip",
    "sha256": "…",
    "installedAt": "2026-10-10T08:00:00Z"
}
```

`url` 不含凭据、查询串和片段。标记缺失或无效（类型、Package 名、版本或引擎不对）时，这个文件夹算“本地文件夹”。

## 使用

### 视图

**依赖图**：按层级（基础层、能力层、适配层、内容层）排列的 Package 卡片，箭头 A → B 表示 B 依赖 A。有问题的卡片带警告图标；双击卡片回到 Package 视图并选中它。可以缩放（`Ctrl` 加滚轮）、拖动和适应窗口。

### 安装、更新、启用、禁用、卸载

- `.uproject` 的缩进、换行、字段顺序、其他字段和 BOM 都会保持。写入是原子的；文件为只读（Perforce 工作区）时，计划会提示先签出。
- 编译用 UBT 的 `Build.bat`，目标平台 Win64，配置取自 `设置 > 编译 > 配置`（`Development` 或 `DebugGame`）。进度取自输出里的 `[n/m]`。只有确实有 C++ 要编译时才有这一步（见 [什么时候还需要编译](https://astreox.github.io/uBotDocs/manager/packages/)）。
- 运行测试对应 `Automation RunTests <本次安装、更新或启用的 Package>`，以无界面方式运行 `UnrealEditor-Cmd.exe`；由 `设置 > 编译 > 编译后运行 Package 测试` 决定是否运行。

## 设置

### 保存在哪里

- `有预编译版本时优先使用` 默认开启。已有的 `settings.json` 里写着 `install.usePrebuilt` 的，保持它的值；没有这个键的，按开启处理。

## 命令行 ubot

- 除 `index validate` 和 `engines` 外，每条命令先刷新（`package` 对它自己的宿主工程这样做）：`syncOnStart` 开着就同步远程索引，`--offline` 只用缓存。`--index` 用给定的文件或地址替换配置里的索引源，可以重复；文件相对当前目录。给定的索引源加载不了会报错退出（`index show` 除外，它用来查看这种情况）。
- `status` 的 `FROM` 列（`info` 里是 `Installed from`）显示 Package 的安装方式：`git v0.1.0`（有本地修改时加 ` (modified)`）、`prebuilt UE 5.5`（标记里记录的引擎版本）、`local folder`。`info` 还有一行 `Prebuilt`，说明这个 Package 的安装或更新现在会不会用预编译包。
