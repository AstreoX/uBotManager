> 从 README 的「命令行 ubot」和「发布 uBot Manager」两节迁出，文字未改动。只有一处链接改了：指向 `release.yml` 的相对路径，改成了从本文件出发的路径。

### 制作和发布预编译包

`ubot package` 在一个临时的纯内容工程里，从源码安装并用启动器引擎编译指定的 Package，再把每个 Package 打成 zip，并给出可以直接写进包索引的 `binaries` 条目。

```
ubot package <Name[@版本]>... --out <目录> [--engine <5.5 | 引擎根目录>] [--url-template <模板>]
             [--update-index <索引文件>] [--with-symbols] [--keep-work]
```

`--index`、`--offline` 照常有效。`package` 不需要打开项目，会忽略 `--project`，也不动你的设置、任务历史和最近的项目。

| 选项 | 说明 |
| --- | --- |
| `<Name[@版本]>` | 要打包的 Package，可以写多个；省略版本取最新的非规划版本。版本从包索引里取 |
| `--out <目录>` | 必填。压缩包和 `.sha256` 文件放在这里 |
| `--engine` | 启动器引擎的版本号（如 `5.5`），或启动器引擎的根目录；省略时用最新的启动器引擎。源码构建的引擎会被拒绝。引擎的 `主版本.次版本` 决定索引里的键和文件名 |
| `--url-template` | 压缩包的下载地址模板，默认 `https://github.com/AstreoX/{repo}/releases/download/{ref}/{file}`。占位符：`{repo}`（仓库名，不含 `.git`）、`{ref}`、`{file}`、`{name}`、`{version}`、`{engine}`。结果必须是 `https://`、`file://` 或绝对路径 |
| `--update-index <文件>` | 把 `binaries` 条目写进这个索引文件，再校验一遍 |
| `--with-symbols` | 压缩包里保留 `*.pdb`、`*.exp`、`*.lib`（默认不含） |
| `--keep-work` | 保留 `<out>\_work` 里的临时工程（默认结束后删除） |

做了什么：

1. 在 `<out>\_work` 里建一个纯内容的宿主工程（`EngineAssociation` 是所选引擎）。`_work` 里放着标记文件 `.ubot-package-work`；已存在的 `_work` 只有带这个标记、或者是空的才会被替换，别的内容不会被动，命令直接报错。
2. 通过普通的计划和任务安装这些 Package：关闭预编译、打开编译，所以它们**以及它们依赖的 Package** 都按索引里的 `ref` 克隆并编译（输出里能看到 `Clone …` 和 `Build …`）。只打包你列出的 Package，要让依赖也有预编译包，把它们一起列出。
3. 检查每个 Package 的 `VersionName` 等于索引里的版本，有 C++ 模块的插件编译后有 `Binaries/Win64/UnrealEditor-*.dll` 和 `UnrealEditor.modules`；然后打成 `<out>\<Name>-<版本>-UE<主.次>-Win64.zip`，旁边写同名的 `.sha256` 文件。
4. 打印每个 Package 的 `binaries` 条目（`url`、`sha256`、`size`）。带 `--update-index` 时同时写进索引文件：只改对应版本的 `binaries[引擎][Win64]`，其余内容保持原有的键顺序和格式（新写入的部分用 4 空格缩进）；结果通不过校验时，索引文件保持原样并报错。

压缩包的内容：只有一个以 Package 命名的顶层文件夹（`UBotSensor/`），里面是 `UBotSensor.uplugin`、`Binaries/Win64`、`Source`、`Resources`、`Config` 等；没有 `.git`、`Intermediate`、`Saved`，默认也没有 `*.pdb`、`*.exp`、`*.lib`。

#### 发布流程

给一个 Package 发布新版本的预编译包：

1. 按 uBotCore 的 [`Index/README.md`](https://github.com/AstreoX/uBotCore/blob/main/Index/README.md) 发布源码版本：改 `.uplugin` 的 `VersionName`，打标签 `v<VersionName>` 并推送，在 `Index/index.json` 里追加这个版本（有 `ref`，暂时没有 `binaries`）。`ubot package` 按索引里的 `ref` 克隆，所以标签必须已经推送。
2. 打包，并把条目写进本地的索引文件。要打包的版本还没有出现在 `main` 上，所以用 `--index` 指向本地副本：

   ```powershell
   $index = '..\..\Plugins\uBot\uBotCore\Index\index.json'
   ubot --index $index package UBotCore UBotSensor UBotROS --out E:\release --update-index $index
   ```

   这一步要完整地编译一遍，耗时取决于机器；需要 git、启动器版本的引擎和 Visual Studio 2022。
3. 把 `E:\release` 里的 zip 和同名的 `.sha256` 上传到各自仓库、对应标签的 GitHub Release（默认模板的地址是 `https://github.com/AstreoX/<仓库>/releases/download/<标签>/<文件名>`），文件名必须和索引里 `url` 的最后一段一致。标签还没有 Release 时用 `gh release create` 创建，已经有或者重新上传时用 `gh release upload --clobber`：

   ```powershell
   gh release create v0.1.2 E:\release\UBotCore-0.1.2-UE5.5-Win64.zip E:\release\UBotCore-0.1.2-UE5.5-Win64.zip.sha256 --repo AstreoX/uBotCore --title "uBotCore 0.1.2" --notes-file notes.md
   gh release upload v0.1.2 E:\release\UBotCore-0.1.2-UE5.5-Win64.zip E:\release\UBotCore-0.1.2-UE5.5-Win64.zip.sha256 --repo AstreoX/uBotCore --clobber
   ```
4. 校验：先确认索引里每个新的 `binaries` 地址都能下载（`curl.exe -sIL <url>` 的最后一个状态应是 200），再运行 `ubot index validate $index`。可以在一个使用启动器引擎的测试工程里再跑 `ubot plan install:UBotROS`，计划里应该出现 `Download` 步骤、没有 `Build`。
5. 提交并推送 uBotCore 的 `Index/index.json`（远程源读取的就是 `main`），再在这个仓库运行 `node scripts/sync-index.mjs` 更新内置副本，提交并推送。

先上传、后推送索引：索引一推送，用户就会开始下载。压缩包重新上传过（字节变了）就必须重新 `--update-index`，否则 SHA-256 和 `size` 对不上，安装会失败。

## 发布 uBot Manager

发布用的安装包只由 GitHub Actions 的 `Release` 工作流（[`.github/workflows/release.yml`](../.github/workflows/release.yml)）在 GitHub 托管的 Windows 机器上构建，不用本地打的包。

1. 先发布还没发布的 Package（见 [发布流程](#发布流程)），然后运行 `node scripts/sync-index.mjs` 更新内置索引副本，提交并推送。构建前会检查这份副本是否最新，过旧就失败。
2. 把版本号改成新版本：根目录 `Cargo.toml` 的 `[workspace.package]`、`src-tauri/tauri.conf.json`、`app/package.json`（以及 `app/package-lock.json` 开头的两处），提交并推送。
3. 打标签 `v<版本>` 并推送。工作流检查三处版本和标签一致，跑前端和 Rust 的测试，检查内置索引副本，构建安装包。
4. 签名接入后，工作流先后提交两个签名请求，先签 `ubot-manager.exe`、再签安装包，每个都要在 SignPath 里批准，一小时内没批准就失败。
5. 工作流最后生成一个草稿 Release，附上 `uBot-Manager_<版本>_x64-setup.exe` 和 `.sha256`。检查后补上更新说明再发布。

手动运行工作流（Actions 页的 *Run workflow*）只构建，安装包留在这次运行的 Artifacts 里，不生成 Release。
