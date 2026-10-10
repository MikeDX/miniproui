// Build static SHA-1 shards for the web ROM lookup.
// Usage:
//   node scripts/build-romdb.mjs --mame /opt/homebrew/bin/mame
//   node scripts/build-romdb.mjs --xml mame.xml

import { spawn } from "node:child_process";
import { createReadStream } from "node:fs";
import { mkdir, rename, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import readline from "node:readline";

const outDir = path.resolve("static-romdb");
const tmpDir = path.resolve("static-romdb.tmp");

const machineRe = /<machine\s+name="([^"]+)"([^>]*)>/;
const cloneRe = /\bcloneof="([^"]*)"/;
const descRe = /<description>([^<]*)<\/description>/;
const romRe = /<rom\b([^>]*)\/>/;
const buildRe = /<mame\b[^>]*\bbuild="([^"]*)"/;

function arg(name) {
  const index = process.argv.indexOf(name);
  return index === -1 ? "" : process.argv[index + 1] || "";
}

function decodeXml(text) {
  return text.replace(/&(#x[0-9a-fA-F]+|#\d+|amp|lt|gt|quot|apos);/g, (_, ent) => {
    if (ent === "amp") return "&";
    if (ent === "lt") return "<";
    if (ent === "gt") return ">";
    if (ent === "quot") return '"';
    if (ent === "apos") return "'";
    const code = ent[1] === "x" ? parseInt(ent.slice(2), 16) : parseInt(ent.slice(1), 10);
    return Number.isFinite(code) ? String.fromCodePoint(code) : _;
  });
}

function attr(body, name) {
  const match = new RegExp(`\\b${name}="([^"]*)"`).exec(body);
  return match ? decodeXml(match[1]) : "";
}

function openXml() {
  const xml = arg("--xml");
  const mame = arg("--mame");
  if (xml && mame) {
    console.error("Pass only one of --xml or --mame.");
    process.exit(1);
  }
  if (xml) return { input: createReadStream(xml), close: async () => {} };
  if (!mame) {
    console.error("Usage: node scripts/build-romdb.mjs --mame <mame> | --xml <mame.xml>");
    process.exit(1);
  }
  const child = spawn(mame, ["-listxml"], { stdio: ["ignore", "pipe", "inherit"] });
  const closed = new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("exit", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`mame -listxml exited ${code}`));
    });
  });
  return { input: child.stdout, close: () => closed };
}

const shards = new Map();
let machines = 0;
let roms = 0;
let version = "";
let setName = "";
let parent = "";
let description = "";

function addRom(body) {
  if (attr(body, "status") === "nodump") return;
  const sha1 = attr(body, "sha1").toLowerCase();
  if (!/^[0-9a-f]{40}$/.test(sha1)) return;
  const name = attr(body, "name");
  const size = Number(attr(body, "size"));
  if (!name || !Number.isFinite(size)) return;
  const crc = attr(body, "crc").toLowerCase();
  const entry = { set: setName };
  if (parent) entry.parent = parent;
  entry.desc = description;
  entry.rom = name;
  entry.size = size;
  if (/^[0-9a-f]{8}$/.test(crc)) entry.crc = crc;

  const prefix = sha1.slice(0, 3);
  let bucket = shards.get(prefix);
  if (!bucket) {
    bucket = new Map();
    shards.set(prefix, bucket);
  }
  let list = bucket.get(sha1);
  if (!list) {
    list = [];
    bucket.set(sha1, list);
  }
  list.push(entry);
  roms += 1;
}

function consume(line) {
  if (!version) {
    const build = buildRe.exec(line);
    if (build) version = build[1].split(" ")[0];
  }
  const machine = machineRe.exec(line);
  if (machine) {
    machines += 1;
    setName = decodeXml(machine[1]);
    parent = decodeXml(cloneRe.exec(machine[2])?.[1] || "");
    description = "";
    if (machines % 5000 === 0) console.error(`machines ${machines}, roms ${roms}`);
    return;
  }
  const desc = descRe.exec(line);
  if (desc && setName) {
    description = decodeXml(desc[1]);
    return;
  }
  const rom = romRe.exec(line);
  if (rom && setName) addRom(rom[1]);
}

const source = openXml();
const lines = readline.createInterface({ input: source.input, crlfDelay: Infinity });
for await (const line of lines) consume(line);
await source.close();

await rm(tmpDir, { recursive: true, force: true });
await mkdir(tmpDir, { recursive: true });

let hashes = 0;
for (let i = 0; i < 4096; i++) {
  const prefix = i.toString(16).padStart(3, "0");
  const bucket = shards.get(prefix);
  const body = {};
  if (bucket) {
    for (const [sha1, entries] of bucket) body[sha1] = entries;
    hashes += bucket.size;
  }
  await writeFile(path.join(tmpDir, `${prefix}.json`), JSON.stringify(body));
}

const meta = {
  mame_version: version || "unknown",
  generated: new Date().toISOString(),
  hashes,
};
await writeFile(path.join(tmpDir, "meta.json"), JSON.stringify(meta));

const previous = `${outDir}.old`;
await rm(previous, { recursive: true, force: true });
await rename(outDir, previous).catch(() => {});
await rename(tmpDir, outDir);
await rm(previous, { recursive: true, force: true });

const { readdir, stat } = await import("node:fs/promises");
const names = await readdir(outDir);
let bytes = 0;
for (const name of names) bytes += (await stat(path.join(outDir, name))).size;
console.log(
  `machines ${machines}, roms ${roms}, unique hashes ${hashes}, output ${(bytes / 1e6).toFixed(1)} MB, MAME ${meta.mame_version}`,
);
