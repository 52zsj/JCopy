#!/usr/bin/env node
/**
 * 从 CHANGELOG.md 抽出「当前版本」那一节，用作 GitHub Release 正文。
 *
 * 用法：
 *   node scripts/changelog-section.mjs                 # 打印正文
 *   node scripts/changelog-section.mjs --check         # 只校验有没有这一节
 *   node scripts/changelog-section.mjs --github-output # 写成 GITHUB_OUTPUT 的 body=...
 */
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const version = JSON.parse(readFileSync(path.join(root, "package.json"), "utf8")).version;
const lines = readFileSync(path.join(root, "CHANGELOG.md"), "utf8").split("\n");

const start = lines.findIndex((line) => line.trim() === `## ${version}`);
if (start === -1) {
  console.error(`✗ CHANGELOG.md 里没有 "## ${version}" 这一节，发版前先补上`);
  process.exit(1);
}

let end = lines.length;
for (let i = start + 1; i < lines.length; i += 1) {
  if (/^##\s/.test(lines[i])) {
    end = i;
    break;
  }
}

const body = lines.slice(start + 1, end).join("\n").trim();
if (!body) {
  console.error(`✗ CHANGELOG 的 ${version} 章节是空的`);
  process.exit(1);
}

if (process.argv.includes("--check")) {
  console.log(`✓ CHANGELOG 有 ${version} 一节（${body.split("\n").length} 行）`);
  process.exit(0);
}

if (process.argv.includes("--github-output")) {
  // GITHUB_OUTPUT 用 delim 语法传多行文本
  const delim = "JCOPY_CHANGELOG_EOF";
  console.log(`body<<${delim}`);
  console.log(body);
  console.log(delim);
  process.exit(0);
}

console.log(body);
