const assert = require("node:assert/strict");
const test = require("node:test");
const { platformAsset } = require("../bin/install.js");

const { version } = require("../package.json");
const asset = (name) => `blora-${version}-${name}`;

test("release asset names match the release workflow", () => {
  assert.equal(platformAsset("linux", "x64"), asset("linux-x86_64.tar.gz"));
  assert.equal(platformAsset("darwin", "x64"), asset("macos-x86_64.tar.gz"));
  assert.equal(platformAsset("darwin", "arm64"), asset("macos-aarch64.tar.gz"));
  assert.equal(platformAsset("win32", "x64"), asset("windows-x86_64.zip"));
});

test("unsupported platforms are rejected", () => {
  assert.throws(() => platformAsset("freebsd", "x64"), /unsupported platform: freebsd/);
});
