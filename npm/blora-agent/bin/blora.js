#!/usr/bin/env node

const { existsSync } = require("node:fs");
const { spawn } = require("node:child_process");
const path = require("node:path");

const binary = path.join(__dirname, "..", "bin", process.platform === "win32" ? "blora.exe" : "blora");
if (!existsSync(binary)) {
  console.error("Blora binary is missing. Reinstall @bloret-crew/blora-agent.");
  process.exit(1);
}

const child = spawn(binary, process.argv.slice(2), {
  stdio: "inherit",
  windowsHide: true,
});
child.on("error", (error) => {
  console.error(`Unable to start Blora: ${error.message}`);
  process.exit(1);
});
child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
  } else {
    process.exit(code ?? 1);
  }
});
