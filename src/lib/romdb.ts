export interface RomMatch {
  set: string;
  parent?: string;
  desc: string;
  rom: string;
  size: number;
  crc?: string;
}

interface RomdbMeta {
  mame_version: string;
  generated: string;
  hashes: number;
}

const shards = new Map<string, Record<string, RomMatch[]>>();
let meta: RomdbMeta | null = null;

export async function romdbAvailable(): Promise<boolean> {
  try {
    const response = await fetch("/romdb/meta.json");
    if (!response.ok) return false;
    meta = (await response.json()) as RomdbMeta;
    return Boolean(meta?.mame_version);
  } catch {
    return false;
  }
}

export async function identifyDump(bytes: Uint8Array): Promise<string[]> {
  const version = await mameVersion();
  const sha1 = await sha1Hex(bytes);
  const crc = crc32Hex(bytes);
  const prefix = sha1.slice(0, 3);
  let shard = shards.get(prefix);
  if (!shard) {
    const response = await fetch(`/romdb/${prefix}.json`);
    if (!response.ok) throw new Error("ROM database lookup failed.");
    shard = (await response.json()) as Record<string, RomMatch[]>;
    shards.set(prefix, shard);
  }
  const hits = shard[sha1] ?? [];
  if (hits.length === 0) {
    return [`Not found in MAME ${version}.`, `SHA-1 ${sha1}`, `CRC32 ${crc}`];
  }
  return [...hits.map(formatMatch), `Identified against MAME ${version}.`];
}

function formatMatch(hit: RomMatch): string {
  const parent = hit.parent ? `, parent ${hit.parent}` : "";
  return `${hit.set}: ${hit.desc}, ROM ${hit.rom}${parent}`;
}

async function mameVersion(): Promise<string> {
  if (meta?.mame_version) return meta.mame_version;
  const ok = await romdbAvailable();
  if (!ok || !meta) throw new Error("ROM database lookup failed.");
  return meta.mame_version;
}

async function sha1Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-1", new Uint8Array(bytes));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function crc32Hex(bytes: Uint8Array): string {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) {
      crc = crc & 1 ? (crc >>> 1) ^ 0xedb88320 : crc >>> 1;
    }
  }
  return ((crc ^ 0xffffffff) >>> 0).toString(16).padStart(8, "0");
}
