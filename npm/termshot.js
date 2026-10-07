#!/usr/bin/env node
// The termshot command of the npm package @momiji-rs/termshot. It runs the
// release binary that this machine's platform package carries; npm installs
// only the optional dependency whose os and cpu match (scripts/npm-stage.sh).
"use strict";
const { spawn } = require("node:child_process");

const PACKAGES = {
  "darwin arm64": "@momiji-rs/termshot-darwin-universal",
  "darwin x64": "@momiji-rs/termshot-darwin-universal",
  "linux x64": "@momiji-rs/termshot-linux-x64",
  "linux arm64": "@momiji-rs/termshot-linux-arm64",
};

const platform = `${process.platform} ${process.arch}`;
const pkg = PACKAGES[platform];
if (!pkg) {
  console.error(`termshot: no build for ${platform}; see https://github.com/momiji-rs/termshot#install`);
  process.exit(1);
}
let bin;
try {
  bin = require.resolve(`${pkg}/termshot`);
} catch {
  console.error(`termshot: ${pkg} is not installed; reinstall without --omit=optional`);
  process.exit(1);
}
// Pass on the signals a terminal or a supervisor sends, so that stopping the
// launcher stops termshot too; then end the way termshot ended.
const SIGNALS = ["SIGHUP", "SIGINT", "SIGQUIT", "SIGTERM"];
const child = spawn(bin, process.argv.slice(2), { stdio: "inherit" });
const forward = (signal) => child.kill(signal);
for (const signal of SIGNALS) process.on(signal, forward);
child.on("error", (err) => {
  console.error(`termshot: ${err.message}`);
  process.exit(1);
});
child.on("exit", (code, signal) => {
  for (const s of SIGNALS) process.off(s, forward);
  if (signal) process.kill(process.pid, signal);
  else process.exit(code);
});
