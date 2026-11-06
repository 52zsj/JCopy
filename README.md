# JCopy


跨平台轻量剪贴板共享工具。让 Mac 与 Windows 之间的复制粘贴，像在同一台机器上一样。

- 需求规格：[docs/01-requirements.md](docs/01-requirements.md)
- 技术栈：Tauri 2 + Rust 平台壳 + TypeScript/React 界面
- 当前版本：0.9.16（本机闭环 + 云盘同步 + 全宽卡片条 + 图片同步）

## 现在能做什么

- 监听系统剪贴板，纯文本自动入库（去重、来源应用标记）
- `⌘⇧V` 从屏幕底部滑出横向卡片条，输入即搜索，`←→` 选卡，`↵` 粘贴
- `⌘P` 收藏、`⌘⌫` 删除、`⌘1`–`⌘9` 直接粘贴第 N 张
- 应用黑名单（密码类应用的内容不入库）
- 菜单栏常驻、暂停监听、清空历史
- 自动粘贴默认关闭：关闭时只写回剪贴板，由你自己 `⌘V`
- 跨设备同步：借云盘目录中继，无需配对，开箱即用

## 跨设备同步

两台设备各自登录同一个 iCloud 账号、装好 iCloud 云盘客户端即可。

1. 两台设备都打开「跨设备同步」，指到同一个云盘目录
2. 完事——不需要任何配对步骤
3. 十秒内两台设备会开始互相同步

同步内容走 `iCloud 云盘/JCopy/`：

```
JCopy/
├── devices/<deviceId>.enc   # 每台设备只写自己的，只读别人的
├── images/<deviceId>-<id>.png  # 图片本体，发件箱里只放文件名引用
```

两个设计决定值得说明：

- **每台设备独立文件**，所以两台机器同时运行也不会互相覆盖（这是 JShell 上踩过的坑）
- **无需配对**：两台登录同一云盘账号即互通，不交换任何密钥/码

**图片同步**由设置里的「图片同步」控制，默认「仅收藏」：

| 选项 | 行为 |
|---|---|
| 不同步 | 图片只留本机 |
| 仅收藏（默认） | 只有 ⌘P 收藏过的图片才进 iCloud |
| 全部 | 所有图片都同步，单张上限 12MB |

图片本体写共享目录、发件箱只带引用；对端拉取时拷回本机并本地重建缩略图。
共享目录里自己写的图片保留 30 天，到期自动清理，只删自己那个前缀的文件。

**只有一条通道：云盘目录。** 不再有局域网直连（mDNS + TCP 那套已经移除）——
它在不同网络、企业防火墙、跨平台上带来的问题比省下的几百毫秒多得多。

**已知限制**：
- 云盘是最终一致的文件夹，不是实时通道。文本通常 5–60 秒，图片要单独传原图，几 MB 常见 10 秒–2 分钟
- 云盘空间不足时**文件写不进云端**（本地目录仍能写），另一台永远收不到；
  设置页会显示同步目录所在卷的剩余空间，状态条低于 200MB 会变红

### 不需要配对

**没有任何配对步骤**：两台设备登录同一个云盘账号、指到同一个目录即可。
每台只写自己的 `devices/<id>.enc`、只读别人的，结构上不会互相覆盖。

发件箱仍然加密（剪贴板里常有密码、token 这类东西，不该明文躺在云盘上），
但密钥由程序内部固定派生，用户不用做任何事。

### 同步不通怎么查

设置 → 跨设备同步里有一块自检：

1. **本机发布 N 条**：`N = 0` 说明发件箱是空的，问题在"没东西可发"
2. **共享目录图片 N 张**：程序实际放进云盘目录的图片数。
   拿它和另一台 `JCopy/images` 里的文件数对比，就知道是云盘没传全还是拉取没成功
3. **发件箱列表**：标红那行就是读不了的那份，后面写着原因

## 安装

从 `release/<版本>/` 取 dmg，拖进「应用程序」。

**首次打开会被 Gatekeeper 拦下** —— 当前包未做 Apple 开发者签名与公证：

- 在「应用程序」里右键 JCopy → 打开，或
- `xattr -dr com.apple.quarantine /Applications/JCopy.app`

首次启动后：

1. 菜单栏出现剪贴板图标（没有 Dock 图标，这是有意的）
2. 复制任意文本，按 `⌘⇧V` 看是否出现在面板里
3. 想开自动粘贴，去「系统设置 → 隐私与安全性 → 辅助功能」勾上 JCopy

**Windows 版**需要在 Windows 机器上执行 `npm run app:build`。Tauri 无法在 macOS 上产出 Windows 安装包，
代码里 Windows 分支（`SendInput`、iCloudDrive 路径探测）已写好，但未经真机编译验证。

## 开发

需要 Rust 1.77+ 与 Node 20+。

```bash
npm install
npm run app          # 开发模式（自动起 Vite + Tauri）
npm run build        # 仅构建前端
npm run app:build    # 打安装包
```

首次编译 Rust 依赖较慢（约 5–15 分钟），之后是增量编译。

### 调试时的小开关

面板在失去焦点时会自动收起，这在调试时很不方便。两个环境变量可以帮忙：

```bash
JCOPY_NO_AUTOHIDE=1 npm run app   # 失焦不隐藏
JCOPY_SHOW_ON_START=1 npm run app # 启动即展开面板
```

两者可以叠加。生产构建下 `JCOPY_SHOW_ON_START` 仍可用（便于排查用户环境），`JCOPY_NO_AUTOHIDE` 只在调试时用。

## 打包与发版

产物统一落在 `release/<版本>/`。

### 本地打包

```bash
npm run dist:mac      # macOS：dmg + zip + 安装说明（在 macOS 上跑）
npm run dist:win      # Windows：NSIS 安装包（在 Windows 上跑）
npm run dist:linux    # Linux：deb + AppImage（在 Linux 上跑）
npm run dist:all      # 当前系统支持的全部
```

底层是 `node scripts/build-all.mjs`，参数同上，另有 `--skip-build` 复用已构建的前端产物，
以及 `--clean` 清理历史版本：

```bash
node scripts/build-all.mjs --mac --skip-build
node scripts/build-all.mjs --clean              # 清理 release/ 历史版本，保留最近 2 个
node scripts/build-all.mjs --clean --keep=3     # 保留最近 3 个
```

**每次打包只产出当前版本**：`release/<版本>/` 里只会出现本次版本的文件。
Tauri 的 `bundle/nsis`、`bundle/deb` 这些目录是累积的，装过旧版本会有多个安装包躺在里面，
所以脚本在构建前会清掉旧产物、采集时再按版本号过滤一次，避免历史版本混进来。

另外 `package.sh` 会比对 `Info.plist` 里的版本与 `package.json` 是否一致——
版本号写在三处（`package.json`、`tauri.conf.json`、`Cargo.toml`），漏改一个就会出现
"文件名叫 0.9.2、里面装的还是 0.9.1"，这里直接报错拦下。

**Tauri 不支持跨系统打包**：Windows 要 NSIS/WiX，Linux 要 webkit2gtk 与 dpkg，都无法从 macOS 交叉出包。
所以在 Mac 上跑 `--win` / `--linux` 会明确提示跳过，而不是打出坏包。

macOS 的 dmg 由 `scripts/package.sh` 用 `hdiutil` 直接造，不走 `tauri build --bundles dmg`——
后者会调 AppleScript 驱动 Finder 摆窗口位置，在没有图形会话时报 `-10004` 并整体失败。

### Windows 打包前置条件

`npx tauri build` 只是外壳，真正干活的是 Rust 工具链。最常见的失败是
`failed to run 'cargo metadata' ... program not found`——**cargo 不在 PATH 里**。

在 PowerShell 里依次确认：

```powershell
rustc -V          # 报错说明还没装 Rust
cargo -V          # 同上
rustup show       # 看默认工具链是否为 x86_64-pc-windows-msvc
where.exe cargo   # 看路径是不是 %USERPROFILE%\.cargo\bin\cargo.exe
```

需要准备三样东西：

1. **Rust**：<https://rustup.rs> 下载 `rustup-init.exe`，安装时选 MSVC 工具链
2. **MSVC 生成工具**：安装器会提示，或单独装 Visual Studio Build Tools 的「使用 C++ 的桌面开发」
   （提供 `link.exe`，cargo 通过 vswhere 自动找，不必进 PATH）
3. **PATH**：包含 `%USERPROFILE%\.cargo\bin`。装完 Rust 必须**重开终端**，
   否则当前窗口看不到新路径

临时救急（只对当前窗口生效）：

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
```

`scripts/build-all.mjs` 会先探 PATH、再探默认安装位置，探到了会自动补进子进程环境并给出提示；
两边都没有时直接打印上面的清单后退出，不会抛一堆 node 堆栈。

## 快捷键

| 按键 | 行为 |
|---|---|
| `⌘⇧V` / `Ctrl+Shift+V` | 唤起 / 收起面板 |
| 直接输入 | 搜索 |
| `←` `→` | 左右移动选中卡片（`↑` `↓` 等效） |
| `↵` | 粘贴 |
| `⌘1`–`⌘9` | 直接粘贴第 N 条 |
| `⌘P` | 收藏 / 取消收藏 |
| `⌘⌫` | 删除该条 |
| `Esc` | 收起面板 |

## 目录

```
src/                 前端（React + TS）
  api.ts             invoke 封装与类型契约
  format.ts          时间 / 应用名 / 体积的纯函数
scripts/
  make-icon.py       生成图标（png / icns / ico / tray），改配色重跑即可
src-tauri/
  src/model.rs       Clip 与 Settings
  src/db.rs          SQLite 访问与淘汰策略
  src/clipboard.rs   监听线程
  src/platform.rs    平台能力：前台应用、模拟粘贴
  src/commands.rs    invoke 命令（白名单）
  src/ui.rs          面板定位与显示
  src/tray.rs        菜单栏 / 托盘
  src/sync/
    crypto.rs        AES-GCM / HKDF 原语，密钥内部固定派生（带单测）
    protocol.rs      发件箱的线格式（带端到端单测）
    engine.rs        推送 / 拉取循环
docs/                需求与设计文档
```

## 路线图

- [x] M1 本机闭环：监听、历史、面板、搜索、粘贴、黑名单、托盘
- [x] M2 iCloud 中继：设备配对、端到端加密、增量同步
- [x] M3 ~~局域网直连~~：0.9.5 起移除，统一走云盘
- [ ] M4 打磨：图片历史、视觉细节、签名与分发
