import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

const version = "0.2.104";

function run(command, args) {
  const result = spawnSync(command, args, { stdio: "inherit" });
  if (result.error) {
    console.error(result.error.message);
    process.exit(1);
  }
  if (result.status !== 0) process.exit(result.status ?? 1);
}

function has(command) {
  const result = spawnSync(command, ["--version"], { stdio: "ignore" });
  return result.status === 0;
}

if (!has("rustup")) {
  console.error("rustup is required to build the WebUSB module.");
  process.exit(1);
}

run("rustup", ["target", "add", "wasm32-unknown-unknown"]);

if (!has("wasm-bindgen")) {
  console.log(`Installing wasm-bindgen-cli ${version}...`);
  run("cargo", ["install", "wasm-bindgen-cli", "--version", version, "--locked"]);
}

run("cargo", [
  "build",
  "--manifest-path",
  "web-core/Cargo.toml",
  "--target",
  "wasm32-unknown-unknown",
  "--release",
]);

const targetRoot = process.env.CARGO_TARGET_DIR || path.join("web-core", "target");
const wasm = path.join(targetRoot, "wasm32-unknown-unknown/release/minipro_web.wasm");
if (!existsSync(wasm)) {
  console.error(`Missing ${wasm}`);
  process.exit(1);
}

const outDir = "src/lib/webusb/pkg";
run("wasm-bindgen", ["--target", "web", "--out-dir", outDir, wasm]);
for (const name of readdirSync(outDir)) {
  if (!name.endsWith(".js")) continue;
  const file = path.join(outDir, name);
  const source = readFileSync(file, "utf8");
  if (!source.startsWith("// @ts-nocheck")) {
    writeFileSync(file, `// @ts-nocheck\n${source}`);
  }
}
