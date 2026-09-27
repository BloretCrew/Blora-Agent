#!/usr/bin/env node

const { createWriteStream, existsSync, mkdirSync, renameSync, rmSync } = require("node:fs");
const { tmpdir } = require("node:os");
const path = require("node:path");
const { pipeline } = require("node:stream/promises");
const { request } = require("node:https");

const version = require("../package.json").version;
const repo = "BloretCrew/Blora-Agent";
const installDir = path.join(__dirname);
const executable = process.platform === "win32" ? "blora.exe" : "blora";
const destination = path.join(installDir, executable);
const DOWNLOAD_TIMEOUT_MS = 120_000;
const MAX_ATTEMPTS = 4;

function platformAsset() {
  const arch = process.arch === "x64" ? "x86_64" : process.arch;
  const platform = process.platform === "win32" ? "windows" : process.platform;
  const ext = platform === "windows" ? "zip" : "tar.gz";
  return `blora-${version}-${platform}-${arch}.${ext}`;
}

function wait(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function download(url, destinationPath, redirects = 0) {
  if (redirects > 8) return Promise.reject(new Error("too many redirects"));
  return new Promise((resolve, reject) => {
    const fail = (error) => {
      try { rmSync(destinationPath, { force: true }); } catch {}
      reject(error);
    };
    const req = request(
      url,
      {
        headers: {
          Accept: "application/octet-stream",
          Connection: "keep-alive",
          "User-Agent": "@bloret-crew/blora-agent",
        },
        timeout: DOWNLOAD_TIMEOUT_MS,
      },
      (response) => {
        if ([301, 302, 303, 307, 308].includes(response.statusCode)) {
          response.resume();
          return download(response.headers.location, destinationPath, redirects + 1).then(resolve, reject);
        }
        if (response.statusCode !== 200) {
          response.resume();
          return fail(new Error(`download failed with HTTP ${response.statusCode}`));
        }
        pipeline(response, output).then(() => resolve(), reject);
      },
    );
    req.on("timeout", () => req.destroy(new Error(`download timed out after ${DOWNLOAD_TIMEOUT_MS / 1000}s`)));
    req.on("error", fail);
  });
}

async function downloadWithRetry(url, destinationPath) {
  let lastError;
  for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt += 1) {
    try {
      await download(url, destinationPath);
      return;
    } catch (error) {
      lastError = error;
      if (attempt < MAX_ATTEMPTS) {
        const delay = 1_000 * 2 ** (attempt - 1);
        console.error(`Download attempt ${attempt} failed (${error.message}); retrying in ${delay / 1000}s…`);
        await wait(delay);
      }
    }
  }
  throw lastError;
}

async function extract(archive, target) {
  if (process.platform === "win32") {
    const { execFile } = require("node:child_process");
    await new Promise((resolve, reject) => {
      execFile(
        "powershell.exe",
        [
          "-NoProfile",
          "-NonInteractive",
          "-Command",
          `Expand-Archive -LiteralPath '${archive.replaceAll("'", "''")}' -DestinationPath '${target.replaceAll("'", "''")}' -Force`,
        ],
        (error) => (error ? reject(error) : resolve()),
      );
    });
    return;
  }
  const { execFile } = require("node:child_process");
  await new Promise((resolve, reject) => {
    execFile("tar", ["-xzf", archive, "-C", target], (error) => (error ? reject(error) : resolve()));
  });
}

async function main() {
  if (process.env.BLORA_SKIP_BINARY_DOWNLOAD === "1" || existsSync(destination)) return;
  const asset = platformAsset();
  const archive = path.join(tmpdir(), `blora-${process.pid}-${asset}`);
  const target = path.join(tmpdir(), `blora-${process.pid}`);
  mkdirSync(target, { recursive: true });
  const url = `https://github.com/${repo}/releases/download/v${version}/${asset}`;
  try {
    console.log(`Downloading Blora ${version} for ${process.platform}/${process.arch}…`);
    await downloadWithRetry(url, archive);
    await extract(archive, target);
    const extracted = path.join(target, executable);
    if (!existsSync(extracted)) throw new Error(`release archive does not contain ${executable}`);
    renameSync(extracted, destination);
    if (process.platform !== "win32") require("node:fs").chmodSync(destination, 0o755);
    console.log("Blora installed.");
  } catch (error) {
    console.error(`Blora binary download failed: ${error.message}`);
    console.error(`Install the binary from https://github.com/${repo}/releases/tag/v${version}`);
    process.exitCode = 1;
  } finally {
    try { rmSync(archive, { force: true }); } catch {}
    try { rmSync(target, { recursive: true, force: true }); } catch {}
  }
}

main();
