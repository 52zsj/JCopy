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
import { copyFileSync, existsSync, mkdirSync, readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const argv = process.argv.slice(2);
const has = (flag) => argv.includes(flag);
const skipBuild = has("--skip-build");

const version = execFileSync(process.execPath, ["-p", "require('./package.json').version"], {
  cwd: root,
  encoding: "utf8",
}).trim();

const host = process.platform; // darwin | win32 | linux
const hostName = { darwin: "mac", win32: "win", linux: "linux" }[host] ?? host;

const requested = has("--all")
  ? ["mac", "win", "linux"]
  : ["mac", "win", "linux"].filter((target) => has(`--${target}`));

if (requested.length === 0) {
  console.log("用法：node scripts/build-all.mjs --mac|--win|--linux|--all [--skip-build]");
  process.exit(1);
}

const outDir = path.join(root, "release", version);
mkdirSync(outDir, { recursive: true });

function run(command, args, options = {}) {
  console.log(`\n$ ${command} ${args.join(" ")}`);
  execFileSync(command, args, { cwd: root, stdio: "inherit", ...options });
}

function collect(fromDir, pattern) {
  if (!existsSync(fromDir)) return [];
  const copied = [];
  for (const name of readdirSync(fromDir)) {
    if (!pattern.test(name)) continue;
    const target = path.join(outDir, name);
    copyFileSync(path.join(fromDir, name), target);
    copied.push(target);
  }
  return copied;
}

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
    // 所以那边用 hdiutil 自己造盘
    run("bash", ["scripts/package.sh"], {
      env: { ...process.env, ...(skipBuild ? { JCOPY_SKIP_BUILD: "1" } : {}) },
    });
    built.push("macOS dmg + zip");
    continue;
  }

  if (!skipBuild) run("npm", ["run", "build"]);

  if (target === "win") {
    run("npx", ["tauri", "build", "--bundles", "nsis"]);
    const files = collect(
      path.join(root, "src-tauri/target/release/bundle/nsis"),
      /-setup\.exe$/,
    );
    if (files.length === 0) {
      console.error("✗ 没找到 NSIS 安装包，检查 src-tauri/target/release/bundle/nsis/");
      process.exit(1);
    }
    built.push(...files.map((f) => path.basename(f)));
  }

  if (target === "linux") {
    run("npx", ["tauri", "build", "--bundles", "deb,appimage"]);
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
