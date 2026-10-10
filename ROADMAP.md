# 开发路线图

uBot Manager 与 uBot 系列 Package 的待办事项。完成一项就勾掉；较大的事项可以拆成 GitHub Issue，并在这里附上链接。

## 已知问题

- [ ] **安装包内置的 index 副本过旧。** uBot Manager 0.1.0 安装包里的内置 index 是在加入预编译下载条目（`binaries`）之前打包的。能连上 GitHub 时没有影响；连不上时，Manager 会退回从源码克隆并编译。下一版发布前先运行 `node scripts/sync-index.mjs`，确认 `node scripts/sync-index.mjs --check` 通过后再打包；最好把这项检查加进打包流程，让副本过旧时打包直接失败。
- [ ] 安装包没有代码签名，首次运行时 SmartScreen 会提示"未知发布者"。
- [ ] 预编译包只含编辑器用的 DLL。把项目打包成游戏时仍需要 Visual Studio 从源码编译。
- [ ] 多客户端 PIE 时，各个世界的 ROS 桥接状态（`ros.bridge`）会互相覆盖。
- [ ] 任务记录按项目名区分项目：不同目录下的同名项目会被当成同一个，影响"重新运行"的判断。

## 尚未验证

- [ ] 在真实的桌面窗口里完整手动点测一遍（目前只核对过截图）。
- [ ] "编译后重新打开编辑器"和"等待编辑器关闭"两个流程，还没有对着真实运行的 UE 编辑器跑过。

## 计划

### uBot Manager 0.1.1

- [ ] 修复上面的内置 index 问题，重新发布安装包。
- [ ] 整理发布清单：版本号 → 打标签 → `ubot package` → `gh release create` → 写入 `binaries` → 同步内置副本。

### 预编译版本

- [ ] 支持更多引擎版本（如 UE 5.6），需要在装有对应启动器引擎的机器上运行 `ubot package --engine 5.6`。

### 第二批 Package

index 里已经登记为"规划中"：

- [ ] UBotVehicles：基于 Chaos Vehicles 的差速与阿克曼地面车辆
- [ ] UBotAerial：多旋翼，速度与姿态控制
- [ ] UBotEnvironment：烟雾、雾等影响感知的环境效果
- [ ] UBotMCP：通过 MCP 向大模型开放传感器与执行器
- [ ] UBotLearning：对接 UE Learning Agents
- [ ] UBotCombat：炮塔、弹药、CIWS 等作战场景内容

### 从旧插件迁移

- [ ] 把 AgentBattle 项目迁移到 uBot。
- [ ] 按 uBot 名称重新生成蓝图节点手册（目前仍是 AgentSensorCore 的名称）。
- [ ] 用一个真实的旧蓝图验证 `IsSensorRunning` 的兼容性。
- [ ] 删除 DevHost 里遗留的 TempoROS 副本（`DevHost/Plugins/TempoROS`）。
