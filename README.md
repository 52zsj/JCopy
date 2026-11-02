# JCopy

跨平台轻量剪贴板共享工具。让 Mac 与 Windows 之间的复制粘贴，像在同一台机器上一样。

- 需求规格：[docs/01-requirements.md](docs/01-requirements.md)
- 技术栈：Tauri 2 + Rust 平台壳 + TypeScript/React 界面
- 当前版本：0.6.0（本机闭环 + iCloud 同步 + 局域网直连 + 全宽卡片条 + 图片）

## 现在能做什么

- 监听系统剪贴板，纯文本自动入库（去重、来源应用标记）
- `⌘⇧V` 从屏幕底部滑出横向卡片条，输入即搜索，`←→` 选卡，`↵` 粘贴
- `⌘P` 收藏、`⌘⌫` 删除、`⌘1`–`⌘9` 直接粘贴第 N 张
- 应用黑名单（密码类应用的内容不入库）
- 菜单栏常驻、暂停监听、清空历史
- 自动粘贴默认关闭：关闭时只写回剪贴板，由你自己 `⌘V`
- 跨设备同步：借 iCloud 云盘中继，端到端加密，6 位配对码

## 跨设备同步

两台设备各自登录同一个 iCloud 账号、装好 iCloud 云盘客户端即可。

1. 设备 A 打开设置 → 跨设备同步 →「生成配对码」，得到 6 位数字
2. 设备 B 打开设置 →「输入配对码」，填入
3. 十秒内两台设备会开始互相同步

同步内容走 `iCloud 云盘/JCopy/`：

```
JCopy/
├── devices/<deviceId>.enc   # 每台设备只写自己的，只读别人的
├── images/<deviceId>-<id>.png  # 图片本体，发件箱里只放文件名引用
└── pair/<inviteId>.invite   # 一次性配对邀请，10 分钟过期
```

两个设计决定值得说明：

- **每台设备独立文件**，所以两台机器同时运行也不会互相覆盖（这是 JShell 上踩过的坑）
- **配对不依赖局域网**：邀请文件放进共享目录，用配对码加密主密钥，两台设备不在同一个网络也能配上

**图片同步**由设置里的「图片同步」控制，默认「仅收藏」：

| 选项 | 行为 |
|---|---|
| 不同步 | 图片只留本机 |
| 仅收藏（默认） | 只有 ⌘P 收藏过的图片才进 iCloud |
| 全部 | 所有图片都同步，单张上限 12MB |

图片本体写共享目录、发件箱只带引用；对端拉取时拷回本机并本地重建缩略图。
共享目录里自己写的图片保留 30 天，到期自动清理，只删自己那个前缀的文件。

**两条通道，自动选择**：

- **局域网直连**（优先，仅文本）：mDNS 广播 `_jcopy._tcp` 发现同网段设备，每条消息建一次 TCP 推送。
  实测同机回环 6ms，真实局域网预计 <20ms。用 `jcopy-lan-v1` 作为 HKDF salt 派生的会话密钥加密，
  与云端密钥互不影响；能解开即证明对方持有主密钥。
- **iCloud 中继**（兜底，文本 + 图片）：发现不到、连接失败、或对方未配对时静默降级，用户无感。

之所以不维护长连接：剪贴板事件是低频的，为每条消息建一次连接（同网段 <20ms）比维护心跳、
断线重连、连接池简单得多，出问题也更好排查。

**已知限制**：iCloud 是最终一致的文件夹，不是实时通道，跨网延迟 5–60 秒且受 iCloud 客户端影响。
图片比文本更慢（大文件传输本身要时间），且不计入局域网直推。
公司网络若禁 mDNS，局域网加速不可用（此时仍可用 iCloud 通道）。

## 安装

从 `release/0.8.1/` 取 dmg 或 GitHub Releases，拖进「应用程序」。

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

底层是 `node scripts/build-all.mjs`，参数同上，另有 `--skip-build` 复用已构建的前端产物：

```bash
node scripts/build-all.mjs --mac --skip-build
```

**Tauri 不支持跨系统打包**：Windows 要 NSIS/WiX，Linux 要 webkit2gtk 与 dpkg，都无法从 macOS 交叉出包。
所以在 Mac 上跑 `--win` / `--linux` 会明确提示跳过，而不是打出坏包。要一次拿全三平台，用下面的 CI。

macOS 的 dmg 由 `scripts/make-dmg.sh` 用 `hdiutil` 直接造，不走 `tauri build --bundles dmg`——
后者会调 AppleScript 驱动 Finder 摆窗口位置，在没有图形会话（CI / 无头）时报 `-10004` 并整体失败。

### 三平台 CI

`.github/workflows/release.yml` 会在三种情况下运行：

| 触发 | 行为 |
|---|---|
| 推 main / PR | 三平台编译 + 单测 + 上传产物为 workflow artifact（不发 Release） |
| 推 `v*` tag | 同上，并创建 GitHub Release、附上三个平台的安装包 |
| 手动 workflow_dispatch | 同推 main |

矩阵：`macos-14`（universal，同时出 Apple Silicon 与 Intel）、`windows-latest`（NSIS）、`ubuntu-22.04`（deb + AppImage）。

### 发版

```bash
npm run release:tag -- --dry-run   # 只做检查，不打 tag
npm run release:tag                # 打 tag 并推送，触发 CI 出包与 Release
```

脚本会先拦住这几类问题，任一不满足直接退出：

1. 工作区有未提交改动
2. 版本号不一致——`package.json` / `src-tauri/tauri.conf.json` / `src-tauri/Cargo.toml` 三处必须相同
   （Tauri 的产物版本取自 `tauri.conf.json`，漏改会打出旧版本号）
3. `CHANGELOG.md` 里没有 `## <当前版本>` 这一节（Release 正文从那里取）
4. tag 已存在（本地或远端）
5. 当前不在 `main` 分支

`CHANGELOG.md` 的写法直接决定 Release 长什么样，用 `npm run notes:section` 可以预览：

```markdown
## 0.8.1

一句话概括这版做了什么。

### 新增
- ...
```

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
src-tauri/
  src/model.rs       Clip 与 Settings
  src/db.rs          SQLite 访问与淘汰策略
  src/clipboard.rs   监听线程
  src/platform.rs    平台能力：前台应用、模拟粘贴
  src/commands.rs    invoke 命令（白名单）
  src/ui.rs          面板定位与显示
  src/tray.rs        菜单栏 / 托盘
  src/sync/
    crypto.rs        AES-GCM / HKDF / Argon2id 原语（带单测）
    protocol.rs      发件箱与配对邀请的线格式（带端到端单测）
    engine.rs        推送 / 拉取循环
docs/                需求与设计文档
```

## 路线图

- [x] M1 本机闭环：监听、历史、面板、搜索、粘贴、黑名单、托盘
- [x] M2 iCloud 中继：设备配对、端到端加密、增量同步
- [x] M3 局域网直连：mDNS 发现 + 加密直推，同网段 <20ms
- [ ] M4 打磨：图片历史、视觉细节、签名与分发
