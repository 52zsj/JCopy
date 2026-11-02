#!/usr/bin/env node
/**
 * 本地打包入口。产物统一落在 release/<版本>/。
 *
 *   node scripts/build-all.mjs --mac              # macOS：dmg + zip + 安装说明
 *   node scripts/build-all.mjs --win              # Windows：NSIS 安装包（需在 Windows 上跑）
 *   node scripts/build-all.mjs --linux            # Linux：deb + AppImage（需在 Linux 上跑）
 *   node scripts/build-all.mjs --all              # 当前系统支持的全部
 *   node scripts/build-all.mjs --mac --skip-build # 复用已构建的前端产物
 *
 * 为什么不能跨平台一次性打完：
 *   Tauri 的打包器依赖本平台的工具链（Windows 要 NSIS/WiX，Linux 要 webkit2gtk 与 dpkg）。
 *   官方不支持从 macOS 交叉打出三平台包。要一次拿全三平台，用 GitHub Actions（.github/workflows/release.yml）。
 */
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, rmSync, statSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const argv = process.argv.slice(2);
const has = (flag) => argv.includes(flag);
const skipBuild = has("--skip-build");

const host = process.platform; // darwin | win32 | linux
const isWin = host === "win32";
const hostName = { darwin: "mac", win32: "win", linux: "linux" }[host] ?? host;

const version = execFileSync(process.execPath, ["-p", "require('./package.json').version"], {
  cwd: root,
  encoding: "utf8",
}).trim();

/* ---------- 工具链预检 ---------- */

/**
 * 找 cargo。
 * Windows 上最常见的事故是 Rust 装好了但终端没重启，PATH 里没有 %USERPROFILE%\.cargo\bin，
 * 表现就是 `cargo metadata ... program not found`。这里先探 PATH，再探默认安装位置。
 */
function locateCargo() {
  try {
    execFileSync("cargo", ["-V"], { stdio: "pipe" });
    return { dir: null, version: execFileSync("cargo", ["-V"], { encoding: "utf8" }).trim() };
  } catch {
    // PATH 里没有，看默认位置
  }

  const binDir = path.join(homedir(), ".cargo", "bin");
  const exe = path.join(binDir, isWin ? "cargo.exe" : "cargo");
  if (existsSync(exe)) {
    const env = { ...process.env, PATH: `${binDir}${path.delimiter}${process.env.PATH ?? ""}` };
    const out = execFileSync(exe, ["-V"], { encoding: "utf8", env }).trim();
    return { dir: binDir, version: out };
  }
  return null;
}

const cargo = locateCargo();
if (!cargo) {
  const lines = [
    "",
    "✗ 找不到 Rust 工具链（cargo），无法打包。",
    "",
    isWin
      ? "Windows 上按顺序检查：\n" +
        "  1) 是否装过 Rust：在 PowerShell 里跑 `rustc -V`，报错就是没装\n" +
        "     安装：https://rustup.rs  → 下载 rustup-init.exe → 一路默认\n" +
        "     安装时必须选 MSVC 工具链，并勾选安装「Visual Studio C++ 生成工具」\n" +
        "  2) 装过但命令找不到：可能是终端在安装之前就打开了，PATH 没刷新\n" +
        "     关掉所有终端重开，或手动执行（当前窗口临时生效）：\n" +
        '       $env:Path = "$env:USERPROFILE\\.cargo\\bin;$env:Path"\n' +
        "  3) 让 PATH 永久生效：把 %USERPROFILE%\\.cargo\\bin 加进「用户环境变量」\n" +
        "  4) 确认默认工具链：`rustup default stable-msvc`"
      : "macOS / Linux 安装：\n" +
        "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh\n" +
        "  装完执行 `source $HOME/.cargo/env` 或重开终端",
    "",
    "确认无误后重新运行本条命令。",
    "",
  ];
  console.error(lines.join("\n"));
  process.exit(1);
}

if (cargo.dir) {
  console.log(`! cargo 不在 PATH 里，本次自动使用 ${cargo.dir}`);
  console.log("  建议把它加进系统 PATH，省得每次都靠脚本兜底");
}

// tauri build 会拉起 cargo，子进程要能继承到
const baseEnv = {
  ...process.env,
  ...(cargo.dir
    ? { PATH: `${cargo.dir}${path.delimiter}${process.env.PATH ?? ""}` }
    : {}),
};

/* ---------- 参数解析 ---------- */

const requested = has("--all")
  ? ["mac", "win", "linux"]
  : ["mac", "win", "linux"].filter((target) => has(`--${target}`));

if (requested.length === 0 && !has("--clean")) {
  console.log(
    "用法：node scripts/build-all.mjs --mac|--win|--linux|--all [--skip-build]\n" +
      "      node scripts/build-all.mjs --clean [--keep=2]   # 清理 release/ 里的历史版本",
  );
  process.exit(1);
}

const outDir = path.join(root, "release", version);
mkdirSync(outDir, { recursive: true });

/**
 * 统一执行入口。
 * Windows 上 node_modules/.bin 里是一堆 .cmd，execFile 直接跑会 ENOENT/EINVAL，
 * 所以 Windows 走 shell；其他平台用 argv 数组，避免多余的一层转义。
 */
function run(command, args, options = {}) {
  const printable = [command, ...args].join(" ");
  console.log(`\n$ ${printable}`);
  const env = { ...baseEnv, ...(options.env ?? {}) };
  if (isWin) {
    const quoted = [command, ...args]
      .map((part) => (/[\s"]/.test(part) ? `"${part.replace(/"/g, '\\"')}"` : part))
      .join(" ");
    execFileSync(quoted, { cwd: root, stdio: "inherit", shell: true, env });
  } else {
    execFileSync(command, args, { cwd: root, stdio: "inherit", env });
  }
}

/** 直接跑 tauri CLI 的 JS 入口，绕开 npx 在 Windows 上的 .cmd 问题 */
function tauri(args) {
  const cli = path.join(root, "node_modules/@tauri-apps/cli/tauri.js");
  if (!existsSync(cli)) {
    console.error("✗ 找不到 @tauri-apps/cli，先执行 npm install");
    process.exit(1);
  }
  run(process.execPath, [cli, ...args]);
}

/**
 * 从构建目录里挑出**本次版本**的产物拷贝到 release/<版本>/。
 *
 * 必须按版本号过滤：Tauri 的 bundle 目录是累积的，装过 0.8.5 再打 0.9.1，
 * `bundle/nsis/` 里会同时躺着 JCopy_0.8.5_x64-setup.exe 和 JCopy_0.9.1_x64-setup.exe。
 * 以前这里是「匹配到就拷」，于是当前版本目录里混进了历史版本安装包
 * ——包名不同、版本号也不同，看起来就像"一次打出好几个版本"。
 */
function collect(fromDir, pattern) {
  if (!existsSync(fromDir)) return [];
  // 匹配 -0.9.1 或 _0.9.1，并且后面紧跟 . _ - 或结束，避免 0.9.1 命中 0.9.10
  const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const versionTag = new RegExp(`(^|[-_])${escaped}([._-]|$)`);

  const copied = [];
  const ignored = [];
  for (const name of readdirSync(fromDir)) {
    if (!pattern.test(name)) continue;
    if (!versionTag.test(name)) {
      ignored.push(name);
      continue;
    }
    const target = path.join(outDir, name);
    copyFileSync(path.join(fromDir, name), target);
    copied.push(target);
  }

  if (ignored.length > 0) {
    console.log(`  ! 跳过 ${ignored.length} 个其他版本的产物：${ignored.join("、")}`);
  }
  return copied;
}

/** 打包前清掉该平台的旧产物，避免历史版本继续堆在 bundle 目录里 */
function cleanBundle(...dirs) {
  let removed = 0;
  for (const dir of dirs) {
    if (!existsSync(dir)) continue;
    for (const name of readdirSync(dir)) {
      // 只删文件形态的产物；JCopy.app 这类目录不能碰
      if (!/\.(exe|deb|AppImage|dmg|rpm)$/i.test(name)) continue;
      try {
        rmSync(path.join(dir, name), { force: true });
        removed += 1;
      } catch {
        // 被占用就先留着，采集时还有版本过滤兜底
      }
    }
  }
  if (removed > 0) console.log(`  · 已清掉构建目录里 ${removed} 个旧产物`);
}

/* ---------- release 目录清理 ---------- */

const releaseRoot = path.join(root, "release");

function dirSize(dir) {
  let total = 0;
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    total += entry.isDirectory() ? dirSize(full) : statSync(full).size;
  }
  return total;
}

function versionDirs() {
  if (!existsSync(releaseRoot)) return [];
  return readdirSync(releaseRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort((a, b) => {
      const pa = a.split(".").map(Number);
      const pb = b.split(".").map(Number);
      for (let i = 0; i < Math.max(pa.length, pb.length); i += 1) {
        const diff = (pb[i] ?? 0) - (pa[i] ?? 0);
        if (diff !== 0) return diff;
      }
      return 0;
    });
}

function summarizeRelease() {
  const dirs = versionDirs();
  if (dirs.length === 0) return;
  const bytes = dirSize(releaseRoot);
  const mb = (bytes / 1024 / 1024).toFixed(0);
  console.log(`\nrelease/ 现存 ${dirs.length} 个版本、共 ${mb} MB（最新 ${dirs[0]}）`);
  if (dirs.length > 3) {
    console.log("  历史版本可用 `node scripts/build-all.mjs --clean` 清理（默认保留最近 2 个）");
  }
}

// --clean：清历史版本，默认保留最近 2 个
if (has("--clean")) {
  const keepArg = argv.find((item) => item.startsWith("--keep="));
  const keep = Math.max(0, Number(keepArg?.split("=")[1] ?? 2) || 0);
  const dirs = versionDirs();
  const drop = dirs.slice(keep);
  if (drop.length === 0) {
    console.log(`release/ 只有 ${dirs.length} 个版本，无需清理`);
  } else {
    let freed = 0;
    for (const name of drop) {
      const full = path.join(releaseRoot, name);
      freed += dirSize(full);
      rmSync(full, { recursive: true, force: true });
      console.log(`  删除 release/${name}`);
    }
    console.log(`✓ 清理 ${drop.length} 个历史版本，释放 ${(freed / 1024 / 1024).toFixed(0)} MB`);
    console.log(`  保留：${dirs.slice(0, keep).join("、") || "无"}`);
  }
  if (requested.length === 0) process.exit(0);
}

/* ---------- 逐目标构建 ---------- */

const built = [];
const skipped = [];

for (const target of requested) {
  if (target !== hostName) {
    skipped.push(
      `${target}：当前系统是 ${hostName}，Tauri 不支持跨系统打包；` +
        `请在 ${target === "mac" ? "macOS" : target === "win" ? "Windows" : "Linux"} 上运行，` +
        "或推 tag 让 GitHub Actions 构建三平台",
    );
    continue;
  }

  if (target === "mac") {
    // 交给 package.sh：内置 dmg 打包器会调 AppleScript 摆窗口，无图形会话时直接失败，
    // 所以那边用 hdiutil 自己造盘。
    // 先清掉 bundle/macos 里历史遗留的 rw.*.dmg（tauri 内置 dmg 打包器失败时的中间镜像）
    cleanBundle(path.join(root, "src-tauri/target/release/bundle/macos"));
    run("bash", ["scripts/package.sh"], {
      env: skipBuild ? { JCOPY_SKIP_BUILD: "1" } : {},
    });
    built.push("macOS dmg + zip");
    continue;
  }

  // 走 npm script：里面含 tsc 类型检查，直接调 vite 会漏掉这一步
  if (!skipBuild) run("npm", ["run", "build"]);

  if (target === "win") {
    cleanBundle(path.join(root, "src-tauri/target/release/bundle/nsis"));
    tauri(["build", "--bundles", "nsis"]);
    const files = collect(path.join(root, "src-tauri/target/release/bundle/nsis"), /-setup\.exe$/i);
    if (files.length === 0) {
      console.error("✗ 没找到 NSIS 安装包，检查 src-tauri/target/release/bundle/nsis/");
      process.exit(1);
    }
    built.push(...files.map((f) => path.basename(f)));
  }

  if (target === "linux") {
    cleanBundle(
      path.join(root, "src-tauri/target/release/bundle/deb"),
      path.join(root, "src-tauri/target/release/bundle/appimage"),
    );
    tauri(["build", "--bundles", "deb,appimage"]);
    const files = [
      ...collect(path.join(root, "src-tauri/target/release/bundle/deb"), /\.deb$/),
      ...collect(path.join(root, "src-tauri/target/release/bundle/appimage"), /\.AppImage$/),
    ];
    if (files.length === 0) {
      console.error("✗ 没找到 deb / AppImage，检查 src-tauri/target/release/bundle/");
      process.exit(1);
    }
    built.push(...files.map((f) => path.basename(f)));
  }
}

console.log("\n==> 完成");
console.log(`产物目录：release/${version}/`);
for (const item of built) console.log(`  ✓ ${item}`);
if (skipped.length > 0) {
  console.log("\n跳过：");
  for (const item of skipped) console.log(`  - ${item}`);
}
summarizeRelease();
