#!/usr/bin/env node
/**
 * 一键发版：打 tag 并推送，触发 GitHub Actions 产出三平台安装包与 Release。
 *
 * 做的事：
 *   1. 工作区必须干净（有未提交改动就停，避免把半成品打出去）
 *   2. 三个地方的版本号必须一致：package.json / tauri.conf.json / Cargo.toml
 *      —— Tauri 的产物版本取自 tauri.conf.json，漏改会打出旧版本号
 *   3. CHANGELOG 必须有当前版本的一节（Release 正文从那里来）
 *   4. tag 不能已存在（本地和远端都查）
 *   5. 必须在 main 分支
 *   6. 创建附注 tag v<版本>，推送 main 与 tag
 *
 * 用法：
 *   node scripts/release-tag.mjs --dry-run   # 只检查，不落任何东西
 *   node scripts/release-tag.mjs             # 真正打 tag 并推送
 */
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const dryRun = process.argv.includes("--dry-run");

const git = (args) =>
  execFileSync("git", args, {
    cwd: root,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  }).trim();

function fail(message) {
  console.error(`✗ ${message}`);
  process.exit(1);
}

const version = JSON.parse(readFileSync(path.join(root, "package.json"), "utf8")).version;
const tag = `v${version}`;

// 1. 工作区干净
const status = git(["status", "--porcelain"]);
if (status) {
  fail(`工作区有未提交改动，先提交再发版：\n${status}`);
}

// 2. 三处版本号一致
const tauriConf = JSON.parse(readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8"));
const cargoToml = readFileSync(path.join(root, "src-tauri/Cargo.toml"), "utf8");
const cargoVersion = (cargoToml.match(/^version = "(.+)"$/m) || [])[1];

if (tauriConf.version !== version) {
  fail(`tauri.conf.json 版本是 ${tauriConf.version}，与 package.json 的 ${version} 不一致`);
}
if (cargoVersion !== version) {
  fail(`Cargo.toml 版本是 ${cargoVersion ?? "(未找到)"}，与 package.json 的 ${version} 不一致`);
}

// 3. CHANGELOG 有这一版
try {
  execFileSync(process.execPath, ["scripts/changelog-section.mjs", "--check"], {
    cwd: root,
    stdio: "pipe",
  });
} catch (error) {
  fail(`CHANGELOG 校验未通过：${(error.stdout || error.stderr || "").toString().trim()}`);
}

// 4. tag 未被占用（本地 + 远端）
if (git(["tag", "--list", tag])) {
  fail(`tag ${tag} 已存在，改版本号或先删除旧 tag`);
}
const remoteTag = execFileSync("git", ["ls-remote", "--tags", "origin", tag], {
  cwd: root,
  encoding: "utf8",
  stdio: ["ignore", "pipe", "pipe"],
}).trim();
if (remoteTag) {
  fail(`远端已有 tag ${tag}，说明这版已经发布过`);
}

// 5. 在 main 上
const branch = git(["rev-parse", "--abbrev-ref", "HEAD"]);
if (branch !== "main") {
  fail(`当前分支是 ${branch}，发版应在 main 上`);
}

if (dryRun) {
  console.log(`✓ 检查通过：将创建 tag ${tag}，并推送 ${branch} 与该 tag。`);
  console.log("  （--dry-run：什么都没做）");
  process.exit(0);
}

git(["tag", "-a", tag, "-m", `JCopy ${version}`]);
console.log(`✓ 已创建 tag ${tag}`);

git(["push", "origin", branch]);
git(["push", "origin", tag]);
console.log(`✓ 已推送 ${branch} 与 ${tag}`);
console.log("");
console.log("GitHub Actions 会开始构建 macOS / Windows / Linux 产物，并创建 Release：");
console.log("  https://github.com/52zsj/JCopy/actions");
console.log("  https://github.com/52zsj/JCopy/releases");
