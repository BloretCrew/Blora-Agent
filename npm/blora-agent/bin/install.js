#!/usr/bin/env node

const { createWriteStream, existsSync, mkdirSync, renameSync } = require("node:fs");
const { homedir, tmpdir } = require("node:os");
const path = require("node:path");
const { pipeline } = require("node:stream/promises");
const { request } = require("node:https");

const version = require("../package.json").version;
const repo = "BloretCrew/Blora-Agent";
const installDir = path.join(__dirname);
const executable = process.platform === "win32" ? "blora.exe" : "blora";
const destination = path.join(installDir, executable);

function platformAsset() {
  const arch = process.arch === "x64" ? "x86_64" : process.arch;
  const platform = process.platform === "win32" ? "windows" : process.platform;
  const ext = platform === "windows" ? "zip" : "tar.gz";
  return `blora-${version}-${platform}-${arch}.${ext}`;
}

function download(url, destinationPath, redirects = 0) {
  if (redirects > 5) return Promise.reject(new Error("too many redirects"));
  return new Promise((resolve, reject) => {
    request(url, { headers: { "User-Agent": "@bloret-crew/blora-agent" } }, (response) => {
      if ([301, 302, 303, 307, 308].includes(response.statusCode)) {
        response.resume();
        return download(response.headers.location, destinationPath, redirects + 1).then(resolve, reject);
      }
      if (response.statusCode !== 200) {
        response.resume();
        return reject(new Error(`download failed with HTTP ${response.statusCode}`));
      }
      const output = createWriteStream(destinationPath);
      pipeline(response, output).then(resolve, reject);
    }).on("error", reject);
  });
}

async function extract(archive, target) {
  if (process.platform === "win32") {
    const { execFile } = require("node:child_process");
    await new Promise((resolve, reject) => {
      execFile("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", `Expand-Archive -LiteralPath '${archive.replaceAll("'", "''")}' -DestinationPath '${target.replaceAll("'", "''")}' -Force`], (error) => error ? reject(error) : resolve());
    });
    return;
  }
  const { execFile } = require("node:child_process");
  await new Promise((resolve, reject) => {
    execFile("tar", ["-xzf", archive, "-C", target], (error) => error ? reject(error) : resolve());
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
    await download(url, archive);
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
    try { require("node:fs").rmSync(archive, { force: true }); } catch {}
    try { require("node:fs").rmSync(target, { recursive: true, force: true }); } catch {}
  }
}

main();
