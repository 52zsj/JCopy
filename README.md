# JCopy

跨平台轻量剪贴板共享工具。让 Mac 与 Windows 之间的复制粘贴，像在同一台机器上一样。

- 需求规格：[docs/01-requirements.md](docs/01-requirements.md)
- 技术栈：Tauri 2 + Rust 平台壳 + TypeScript/React 界面
- 当前版本：0.1.0（M1 本机闭环）

## 现在能做什么

- 监听系统剪贴板，纯文本自动入库（去重、来源应用标记）
- `⌘⇧V` 唤起面板，输入即搜索，`↑↓` 选择，`↵` 粘贴
- `⌘P` 固定、`⌘⌫` 删除、`⌘1`–`⌘9` 快速选取
- 应用黑名单（密码类应用的内容不入库）
- 菜单栏常驻、暂停监听、清空历史
- 自动粘贴默认关闭：关闭时只写回剪贴板，由你自己 `⌘V`

跨设备同步（iCloud 中继 + 局域网直连）尚未接入，见下方路线图。

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

面板在失去焦点时会自动收起，这在调试时很不方便。需要让它常驻时：

```bash
JCOPY_NO_AUTOHIDE=1 npm run app
```

## 快捷键

| 按键 | 行为 |
|---|---|
| `⌘⇧V` / `Ctrl+Shift+V` | 唤起 / 收起面板 |
| 直接输入 | 搜索 |
| `↑` `↓` | 移动选中 |
| `↵` | 粘贴 |
| `⌘1`–`⌘9` | 直接粘贴第 N 条 |
| `⌘P` | 固定 / 取消固定 |
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
docs/                需求与设计文档
```

## 路线图

- [x] M1 本机闭环：监听、历史、面板、搜索、粘贴、黑名单、托盘
- [ ] M2 iCloud 中继：设备配对、端到端加密、增量同步
- [ ] M3 局域网直连：mDNS + TLS，延迟 < 300ms
- [ ] M4 打磨：图片历史、视觉细节、签名与分发
