import { test } from "node:test";
import assert from "node:assert/strict";
import { detectKind, KIND_META } from "../src/format.ts";

test("图片条目直接判为 image", () => {
  assert.equal(detectKind("image", "1024 × 1024"), "image");
});

test("链接认 http/https/www", () => {
  assert.equal(detectKind("text", "https://example.com/a?b=1"), "link");
  assert.equal(detectKind("text", "www.example.com"), "link");
});

test("邮箱不会被当成文本", () => {
  assert.equal(detectKind("text", "zhang.san@example.com"), "mail");
});

test("色值两种写法都识别", () => {
  assert.equal(detectKind("text", "#FF0000"), "color");
  assert.equal(detectKind("text", "rgba(0, 0, 0, 0.5)"), "color");
});

test("以花括号/方括号开头视为代码", () => {
  assert.equal(detectKind("text", '{"a": 1}'), "code");
  assert.equal(detectKind("text", "[1, 2, 3]"), "code");
});

test("普通句子仍是纯文本", () => {
  assert.equal(detectKind("text", "今天下午三点开会"), "text");
  assert.equal(detectKind("text", "   "), "text");
});

test("每种类型都有合法色值，色带不会变黑", () => {
  for (const kind of ["text", "image", "link", "color", "code", "mail"]) {
    assert.match(KIND_META[kind].color, /^#[0-9a-f]{6}$/i, `${kind} 缺少颜色`);
    assert.ok(KIND_META[kind].label.length > 0, `${kind} 缺少标签`);
  }
});
