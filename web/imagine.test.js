const assert = require("node:assert");
const { imagineBoard } = require("./imagine.js");

const hash = "a".repeat(64);
const session = "sess-1";
const board = imagineBoard([
  { kind: "user", text: "一张珊瑚壁纸\n画幅 16:9，张数 1" },
  {
    kind: "tools",
    text: `.blora/images/${session}/${hash}.png image/png 16x9`,
  },
  { kind: "routing", text: "openai → mock" },
  { kind: "tools", text: "image API HTTP 401: unauthorized" },
]);

assert.strictEqual(board.cards.length, 1);
assert.strictEqual(board.cards[0].prompt, "一张珊瑚壁纸");
assert.strictEqual(board.cards[0].name, `${hash}.png`);
assert.strictEqual(board.errors.length, 1);
assert.match(board.errors[0], /HTTP 401/);

const codeLike = imagineBoard([
  { kind: "user", text: "看看文件" },
  { kind: "tools", text: "列出了 3 个目录" },
]);
assert.deepStrictEqual(codeLike.cards, []);
assert.deepStrictEqual(codeLike.errors, []);

console.log("imagine board ok");
