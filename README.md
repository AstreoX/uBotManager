# uBot Manager

uBot 家族的 Package 管理器：一个 Windows 桌面程序，在 UE 5.5 工程里安装、更新、启用、禁用和卸载 uBot Package，处理它们之间的依赖，并在需要时编译工程。同一套核心也提供命令行 `ubot`，用于脚本和 CI。

## 简介

uBot Package 就是 `.uplugin` 带有顶层 `"UBot"` 对象的 UE 插件，格式见 [uBotCore](https://github.com/AstreoX/uBotCore) 的 README。uBot Manager 用 git 管理它们：

| 操作 | 做了什么 |
| --- | --- |
| 安装 | 克隆 Package 的仓库，检出包索引里该版本的 `ref`（git 标签），在 `.uproject` 里启用；缺少的依赖和所需的引擎插件一起处理 |
| 更新、降级 | 在已有的 git 检出里 `fetch` 标签，再检出目标版本的 `ref` |
| 启用、禁用 | 只修改 `.uproject` 的 `Plugins` 列表；禁用时连带禁用依赖它的 Package |
| 卸载 | 删除 `.uproject` 里的条目，把 Package 文件夹移到回收站 |

几个边界：

- 引擎插件（如 `ChaosVehiclesPlugin`）只会在 `.uproject` 里启用，不会安装；引擎里没有的会报错。
- 设置里的“有预编译版本时优先使用”是预留项，目前不可选，也没有任何效果；Package 总是从源码安装，随后编译。
- 编译、运行测试和重新打开编辑器调用的是 UBT 的 `Build.bat` 和引擎里的 Windows 可执行文件，引擎位置也来自 Windows 注册表，所以目前只支持 Windows。

Package 是什么、有哪些依赖和版本约束，由包索引（格式 2）和各个 `.uplugin` 决定，格式见 uBotCore 仓库的 [`Index/README.md`](https://github.com/AstreoX/uBotCore/blob/main/Index/README.md)。编辑器里的 **uBot** 面板（`Window > uBot`）只显示状态并启动 uBot Manager，详见 uBotCore 的 README。

## 运行要求

- Windows 10 或 11；界面基于 WebView2，Windows 11 已自带运行库。
- UE 5.5，启动器版本或源码版本都可以。
- git，在 `PATH` 里，或在 `设置 > 安装` 中指定可执行文件。克隆私有仓库需要事先配好 SSH key 或 credential helper，uBot Manager 运行 git 时禁用了交互式提示。
- 编译需要 Visual Studio 2022（含 C++ 工具链）。Recipe 对话框会检查它是否存在。

## 启动

```powershell
ubot-manager.exe                                   # 打开上次使用的项目
ubot-manager.exe --project E:\Sims\MyRobotSim\MyRobotSim.uproject
```

- 从编辑器启动：`Window > uBot` 里的 *Open uBot Manager* 会带着 `--project` 启动它。uBot Manager 每次启动时把自己的路径写入 `HKCU\Software\AstreoX\uBot Manager` 的 `Executable` 值，编辑器据此找到它。
- 只允许一个实例。再次启动时，新进程把 `--project` 交给已在运行的实例，由它打开该项目并回到前台。
- `ubot-manager.exe` 可以放在任意位置。编辑器找不到它时会弹出通知，指向 <https://github.com/AstreoX/uBotManager/releases>。
- 首次启动的语言跟随系统界面语言，是中文就用简体中文，否则用英文。主题默认深色。

### 项目

项目就是一个已存在的 `.uproject` 文件。

- 打开：Package、依赖图、任务三个视图标题栏右侧的 ⋮ 菜单里的 `打开项目…`（文件对话框，按 `.uproject` 过滤），或没有项目时中间的 `打开…` 按钮。
- 最近的项目：菜单和空白页都会列出，最多 10 个。下次启动自动打开上一个，前提是文件还在。
- 菜单里还有 `在资源管理器中显示` 和 `在 UE 编辑器中打开`（走 `.uproject` 的文件关联）。
- 有任务在运行时不能切换项目，因为任务是针对它计划时的那个项目执行的。

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

### 索引源

`设置 > Package 索引` 里有一张有序的索引源列表，前面的优先：

| 索引源 | 位置 | 说明 |
| --- | --- | --- |
| uBotCore | `https://raw.githubusercontent.com/AstreoX/uBotCore/main/Index/index.json` | 内置，通过 HTTPS 下载，超时 10 秒，遵循 `HTTPS_PROXY` 和 `HTTP_PROXY` |
| 本地副本 | 工程里已安装的 UBotCore 的 `Index/index.json` | 内置；没有安装 UBotCore 时不可用，这不算故障 |
| 自己添加的 | `https://` 地址或文件路径 | 文件路径可以是绝对路径，也可以相对项目目录；`http://` 只接受本机地址 |

内置的两项只能调整顺序，不能删除或禁用。同一个 Package 出现在多个索引源里时，列表中靠前的那个整体生效；Recipe 按 `id` 合并，规则相同。

### 失败与回退

- 远程索引下载成功后缓存到 `%LOCALAPPDATA%\AstreoX\uBot Manager\cache\index-ubot.json`，下载失败时使用缓存；失败原因可以在标题栏刷新按钮的提示里看到。
- 没有缓存时，靠后的索引源（一般是本地副本）顶上；所有索引源都没有给出索引时，使用编译进程序的内置副本（由 `scripts/sync-index.mjs` 从 uBotCore 的 `Index/index.json` 复制）。
- 加载失败的索引源会在问题列表的 `Package 索引` 组里出现。用了缓存、或内置源失败而索引仍然可用时，只是提示；自己添加的源失败又没有缓存，或完全没有索引可用时，是警告。每条都可以忽略。
- 索引文件超过 8 MiB、格式版本不是 2、或内容不符合格式都会被拒绝，原因写在问题里。

### 同步时机

启动时先用本地文件和缓存把界面显示出来，再同步远程索引（`设置 > Package 索引 > 启动时同步索引`，默认开启）。之后点标题栏的刷新按钮会重新下载并重新扫描项目；悬停可以看到每个索引源的状态。做计划时只用本地的索引数据，不会重新下载。

每次刷新后，uBot Manager 把合并后的索引写入 `<项目>/Saved/uBot/index.json`，编辑器里的 uBot 面板读取它；内容没有变化时不重写。

## 使用

### 视图

左侧的工具窗口栏依次是 Package、依赖图，下方是问题和设置（齿轮）。任务从状态栏右侧打开。

**Package**：左边的列表分“已安装”和“uBot 索引”两组，可以按名称或提供的能力搜索；规划中的 Package 可以在 `设置 > Package 索引` 里隐藏。右边是详情：版本下拉框、状态、操作按钮、依赖及其满足情况、提供的能力、仓库链接。Package 有问题时，详情顶部的横幅给出第一个问题和它的快速修复。

**依赖图**：按层级（基础层、能力层、适配层、内容层）排列的 Package 卡片，箭头 A → B 表示 B 依赖 A。有问题的卡片带警告图标；双击卡片回到 Package 视图并选中它。可以缩放（`Ctrl` 加滚轮）、拖动和适应窗口。

**问题**：分“Package”和“上次运行”两个标签，按来源分组。选中一条看事实和快速修复。状态栏只统计错误和警告，信息级的问题（例如 ROS 桥接连不上）不计数。

**任务**：左边是任务及其步骤，右边是日志。可以停止正在运行的任务，或重新运行已结束的任务（仅限当前项目的任务）。

### 安装、更新、启用、禁用、卸载

在 Package 视图里选中一个 Package，点它的按钮。按钮不会立即执行，而是把更改加入待应用列表（每个 Package 最多一项，新的覆盖旧的），状态栏显示 `1 项待应用：安装 UBotROS 0.1.0` 和 `应用…`、`放弃`。再点同一个按钮（此时写着“取消安装”等）可以撤销这一项。

`应用…` 先让 uBot Manager 做计划，打开 `应用更改` 对话框，确认后点 `应用`。

- 顶部列出错误（让计划无法应用）和警告。常见的有：Package 不是 git 检出或有本地修改（不能更新）、在分支上（更新后会切换到标签）、依赖的版本冲突、安装目录已存在、引擎里没有所需的插件。
- 步骤按组排列：`下载`（克隆、检出，依赖在前）、`项目`（改 `.uproject`，包括移除）、`编译`、`完成后`（运行测试、重新打开编辑器）。底栏显示 `4 个步骤 · 需要编译`。
- 依赖栏列出这次更改带来的依赖是否满足；选项里可以勾选 `编译后重新打开编辑器` 和 `保存为 Recipe`（任务成功后，把项目里启用的 uBot Package 集合存成你自己的 Recipe）。

细节：

- 安装目录由 `设置 > 安装 > 安装目录` 决定，默认 `Plugins/uBot`。它必须在工程的 `Plugins` 文件夹或 `AdditionalPluginDirectories` 的某一项之内，否则找不到安装的 Package，计划会报错。文件夹名取自索引条目的 `folder`，没有就取仓库地址的最后一段。
- 更新、降级前，Package 必须是干净的 git 检出。检出的 `.git/config` 如果配置了会让 git 运行外部程序的项（过滤器、`core.sshCommand`、自定义 credential helper 等），uBot Manager 拒绝操作，因为这个文件可能是随工程带来的。
- 依赖按版本约束解析：已安装但版本不满足时更新到满足约束的最高版本，没装的装最高的满足版本，已装但被禁用的启用。
- `.uproject` 的缩进、换行、字段顺序、其他字段和 BOM 都会保持。写入是原子的；文件为只读（Perforce 工作区）时，计划会提示先签出。
- 编译用 UBT 的 `Build.bat`，目标平台 Win64，配置取自 `设置 > 编译 > 配置`（`Development` 或 `DebugGame`）。进度取自输出里的 `[n/m]`。
- 运行测试对应 `Automation RunTests <本次安装、更新或启用的 Package>`，以无界面方式运行 `UnrealEditor-Cmd.exe`；由 `设置 > 编译 > 编译后运行 Package 测试` 决定是否运行。

UE 编辑器正在运行此项目时，除克隆以外的步骤要等编辑器关闭后才能执行。`设置 > 编译 > 应用前等待 UE 编辑器关闭`（默认开启）打开时，这些步骤显示“等待 UE 编辑器关闭”，对话框里的 `关闭编辑器` 会请求编辑器正常关闭（它可能提示保存）；关闭时，对话框提示先关闭编辑器，否则这些步骤会失败。

### Recipe

Recipe 是一组 Package 的组合，比如“ROS 2 传感器”就是 UBotCore、UBotSensor 和 UBotROS。Recipe 来自索引，也来自你在 `应用更改` 里保存的。

点标题栏的 `Recipe` 打开 `应用 Recipe`：

1. 左边选 Recipe。含有规划中 Package 的 Recipe 显示为灰色，带“规划中”标记。
2. 选项目（可以直接输入路径或点浏览）。引擎一行显示该项目用的 UE 版本。
3. Package 列表里勾选要包含的；UBotCore 固定勾选。可以在 Recipe 之外多选或去掉别的 Package。
4. `环境要求` 检查 Visual Studio 2022 和 Git；Recipe 声明了 `rosBridge` 时会列出 `ubot_ros_bridge`，但它在 ROS 端运行，这里不检查。
5. `查看更改…` 打开 `应用更改`，看到的就是普通的安装或启用计划。

应用 Recipe 只做两件事：安装缺少的 Package，启用已安装但被禁用的。已安装的版本不会被更新，别的 Package 也不会被禁用。

### 任务

一次只运行一个任务。任务的每一步按顺序执行，输出实时进入日志（git 和 UBT 的输出不翻译，便于搜索报错）。

- 某一步失败时，后面的步骤跳过，已经完成的步骤不会回滚；中途取消的克隆会删除不完整的文件夹。
- 点停止、关闭程序（最多等 10 秒）、或在命令行按 `Ctrl+C`，都会结束当前步骤的整个进程树，剩余步骤标为已跳过。
- 保留最近 30 个任务，每步最多 5000 行日志，存在 `tasks.json`。程序退出时仍在运行的任务，下次启动标为失败。
- 任务结束后自动刷新项目状态。

### 问题与快速修复

问题在每次刷新时重新计算。只有已启用的 Package 产生依赖类问题。

| 问题 | 级别 | 快速修复 |
| --- | --- | --- |
| 依赖的版本不满足 | 警告 | 将依赖更新到满足约束的最高版本；禁用此 Package |
| 依赖没有安装 | 警告 | 安装依赖（索引里有时）；禁用此 Package |
| 依赖被禁用 | 警告 | 启用依赖；禁用此 Package |
| 需要的引擎插件被禁用 | 警告 | 启用该引擎插件 |
| 引擎里没有需要的插件 | 错误 | 忽略 |
| 旧插件（如 `AgentSensorCore`）与取代它的 Package 同时启用 | 警告 | 禁用旧插件；忽略 |
| 描述文件（`.uplugin`）有误 | 警告 | 无 |
| 依赖循环 | 错误 | 无 |
| 找不到项目使用的引擎 | 错误 | 打开编译设置 |
| 索引源加载失败 | 信息或警告 | 忽略 |
| ROS 桥接连不上 | 信息 | 打开 ROS 设置 |

“忽略”把这个问题对当前项目隐藏（存在 `settings.json` 里）。依赖类的快速修复不会立即执行，而是进入待应用列表，由你确认。

`上次运行` 标签显示编辑器写下的 `Saved/uBot/status.json`：UBotCore 在一次 PIE 会话（或打包后游戏的一次运行）结束时，把各个 Package 在运行中报告的问题写进去，例如 UBotROS 没能连上 ROS 桥接。每条带上会话的时间。

ROS 桥接的检查在每次刷新时进行，前提是 UBotROS 已启用且打开了 `设置 > ROS 桥接 > 刷新时检查桥接`：向工程配置的地址发起 TCP 连接，2 秒内连不上就给出信息级的问题。

## 设置

`设置` 对话框有五个分类，带搜索框。确定或应用后保存；主题在选择时就预览，取消会恢复。

| 分类 | 项 | 默认 |
| --- | --- | --- |
| 外观 | 主题：深色、浅色、跟随系统 | 深色 |
| 外观 | 语言：English、简体中文，重启后生效 | 跟随系统界面语言 |
| Package 索引 | 索引源列表（添加、移除、上移、下移，勾选启用） | 两个内置源 |
| Package 索引 | 启动时同步索引 | 开 |
| Package 索引 | 显示规划中的 Package | 开 |
| 安装 | 安装目录 | `Plugins/uBot` |
| 安装 | Git 可执行文件，旁边的 `测试` 运行 `git --version` | `git` |
| 编译 | 引擎 | 项目使用的引擎 |
| 编译 | 配置：Development、DebugGame | `Development` |
| 编译 | 应用前等待 UE 编辑器关闭 | 开 |
| 编译 | 编译后重新打开编辑器 | 开 |
| 编译 | 编译后运行 Package 测试 | 关 |
| ROS 桥接 | 主机、端口，`测试连接` | `127.0.0.1`、`8268` |
| ROS 桥接 | 刷新时检查桥接 | 开 |

Git 可执行文件必须是 git 本身，不能是 `cmd`、PowerShell、`bash` 或 `.bat` 脚本。

### 保存在哪里

| 内容 | 位置 |
| --- | --- |
| 设置 | `%APPDATA%\AstreoX\uBot Manager\settings.json` |
| 任务历史 | `%LOCALAPPDATA%\AstreoX\uBot Manager\tasks.json` |
| 索引缓存 | `%LOCALAPPDATA%\AstreoX\uBot Manager\cache\` |
| 这个程序的路径（供编辑器查找） | `HKCU\Software\AstreoX\uBot Manager` 的 `Executable` |
| 项目的 ROS 桥接地址 | `<项目>/Config/DefaultUBotROS.ini` 的 `[/Script/UBotROS.UBotROSSettings]`，键 `BridgeHost`、`BridgePort` |
| 合并后的索引（写） | `<项目>/Saved/uBot/index.json` |
| 运行状态（读） | `<项目>/Saved/uBot/status.json` |

- 设置了环境变量 `UBOT_MANAGER_HOME` 时，设置、任务历史和缓存都放在它指向的目录下（`settings.json`、`tasks.json`、`cache\`），并且不写注册表。测试和 CI 应该设置它。
- 工程里装了 UBotROS 时，ROS 的主机和端口是这个工程自己的：读写 `Config/DefaultUBotROS.ini`（这也是 UE 读取的文件，写在 `DefaultEngine.ini` 里的同名段不生效），只改这两行。没有 UBotROS 时用 `settings.json` 里的值。
- `settings.json` 里不认识的键会保留。某个键的类型不对只会让它自己回到默认值。整个文件无法解析时使用默认设置，并在下次保存前把原文件复制为 `settings.json.bad-<时间戳>`。
- `设置 > 安装 > 有预编译版本时优先使用` 目前是灰的。

## 命令行 ubot

`ubot` 与桌面程序用同一个核心、同一份设置、缓存和任务历史。

```
ubot [--project <.uproject>] [--index <文件或地址>]... [--offline] [--json] <命令>
```

| 命令 | 说明 |
| --- | --- |
| `status` | 项目、引擎、Package、其他插件、问题和索引源 |
| `info <Package>` | 一个 Package 的详情和相关问题 |
| `problems` | 问题列表，带事实和快速修复 |
| `plan <更改>...` | 只显示计划，不执行；有错误时退出码为 1 |
| `apply <更改>... [--no-build] [--run-tests] [--reopen-editor] [--yes]` | 计划并执行 |
| `recipes` | Recipe 列表 |
| `recipe <id> [--no-build] [--yes]` | 应用 Recipe |
| `engines` | 已安装的 UE 引擎 |
| `index validate <文件>` | 校验索引文件，有错误时退出码为 1 |
| `index show` | 各索引源的状态和合并后的索引 |

更改写成 `kind:Name[@版本]`：`install:UBotROS`、`install:UBotROS@0.1.0`、`update:UBotSensor@0.1.1`、`enable:UBotSensor`、`disable:UBotSensor`、`uninstall:UBotROS`。只有 `install` 和 `update` 带版本，`update` 必须写明版本。

```powershell
# 看一个项目的状态
ubot --project E:\Sims\MyRobotSim\MyRobotSim.uproject status

# 先看计划
ubot --project E:\Sims\MyRobotSim\MyRobotSim.uproject plan install:UBotROS
```

```
Download
  1. Clone UBotCore 0.1.0  (github.com/AstreoX/uBotCore → Plugins/uBot/uBotCore)
  2. Clone UBotSensor 0.1.0  (github.com/AstreoX/uBotSensor → Plugins/uBot/uBotSensor)
  3. Clone UBotROS 0.1.0  (github.com/AstreoX/uBotROS → Plugins/uBot/uBotROS)
Project
  4. Enable UBotCore  (required by UBotROS)
  5. Enable UBotSensor  (required by UBotROS)
  6. Enable UBotROS  (MyRobotSim.uproject)
Build
  7. Build UnrealEditor  (Win64 · Development)
```

这个例子来自一个纯内容工程，所以编译目标是 `UnrealEditor`；有 `Source/*.Target.cs` 的工程在这一步是 `Build <项目名>Editor`。

```powershell
# 执行；--yes 不再询问，--no-build 跳过编译
ubot --project E:\Sims\MyRobotSim\MyRobotSim.uproject apply install:UBotROS --no-build --yes

# 之后省略 --project，用的就是上次的项目：更新、禁用、卸载
ubot apply update:UBotSensor@0.1.1 --yes
ubot apply disable:UBotSensor --yes
ubot apply uninstall:UBotROS --yes

# Recipe
ubot recipes
ubot recipe ros-sensors --yes

# 校验索引（CI 里用）
ubot index validate Index/index.json
```

- 没有 `--project` 时用上次打开的项目；带 `--project` 会同时把它记为上次的项目。
- 除 `index validate` 和 `engines` 外，每条命令先刷新：`syncOnStart` 开着就同步远程索引，`--offline` 只用缓存。`--index` 用给定的文件或地址替换配置里的索引源，可以重复；文件相对当前目录。给定的索引源加载不了会报错退出（`index show` 除外，它用来查看这种情况）。
- 只有 `apply` 和 `recipe` 会写入项目的 `Saved/uBot/index.json`，只读命令不改动项目。
- `apply` 先打印计划，有错误时以 1 退出；没有 `--yes` 时在标准输入询问 `Apply these changes? [y/N]`。执行时逐步打印 `[n/m] 步骤: 状态`，并缩进显示步骤输出。`--run-tests` 和 `--reopen-editor` 默认关闭，命令行不读取设置里对应的两项。
- 退出码：成功 0，失败 1，被取消 130。`Ctrl+C` 取消正在运行的任务，再按一次立即退出。
- `--json` 输出 JSON：`status` 是完整的快照，`plan` 是计划，`apply` 和 `recipe` 是结束后的任务，`index show` 里的仓库地址不含凭据。
- 输出是英文。仓库地址里内嵌的凭据（`https://user:token@host`）在所有输出和日志里显示为 `https://***@host`。
- `UBOT_MANAGER_PERMANENT_DELETE=1` 让 `uninstall` 直接永久删除文件夹而不是移到回收站，供测试和 CI 使用；桌面程序没有这个开关。

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

- `npm run tauri build` 由 Tauri CLI 驱动：先运行 `npm run build` 生成 `app/dist`，再编译外壳并把前端嵌入。不要直接用 `cargo build` 构建 `ubot-manager`，那样得到的是开发构建，会去加载 `http://localhost:5173`。
- 去掉 `--no-bundle` 会按 `tauri.conf.json` 生成 NSIS 安装包（按当前用户安装）。
- 开发：在 `app/` 里运行 `npm run tauri dev` 启动带热更新的桌面程序；`npm run dev` 只启动前端，页面使用内存里的 mock 后端，可以用参数直达某个界面，例如 `http://localhost:5173/?lang=zh-CN&theme=light&view=problems`，参数有 `lang`、`theme`、`view`、`dialog`、`section`、`select`、`noproject`。

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
- 更新内置的索引副本：`node scripts/sync-index.mjs`（`--check` 只比较不写入）。默认从工作区里的 `Plugins/uBot/uBotCore/Index/index.json` 复制，也可以用 `--from` 或环境变量 `UBOTCORE_INDEX` 指定。
- 手动运行时设置 `UBOT_MANAGER_HOME` 指向一个临时目录，避免动到自己的设置。

## 仓库结构

| 路径 | 内容 |
| --- | --- |
| `crates/ubot-core` | 核心库：索引、项目模型、版本约束、问题、计划、任务执行、设置。`assets/index.json` 是内置的索引副本 |
| `crates/ubot-cli` | 命令行 `ubot` |
| `src-tauri` | Tauri 外壳：命令、事件、单实例、启动流程、注册表登记 |
| `app` | 前端（Vite、React、TypeScript）：视图、对话框、中英文语言包、mock 后端 |
| `scripts` | `sync-index.mjs` |
| `docs/SPEC.md` | 实现所依据的技术规格，英文 |

## 版本与许可

当前版本 **0.1.0**，仓库 <https://github.com/AstreoX/uBotManager>。

Copyright 2026 AstreoX。本项目以 [Apache License 2.0](LICENSE) 发布，可以自由使用、修改和再分发（包括商业用途），再分发时需保留 `LICENSE` 与 `NOTICE` 文件，并注明你修改过的文件。
