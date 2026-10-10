import {
  DATA_OFFSET,
  H_ACTUAL,
  H_ACTUAL_B,
  H_EP_A,
  H_EP_B,
  H_LEN_A,
  H_LEN_B,
  H_OP,
  H_STATE,
  H_STATUS,
  OP_IN,
  OP_IN_PAIR,
  OP_OUT,
  OP_OUT_PAIR,
  PROGRAMMER_FILTERS,
  RECV_TIMEOUT_MS,
  SAB_SIZE,
  SEND_TIMEOUT_MS,
  STATE_IDLE,
  STATE_REQ,
  STATE_RESP,
} from "./protocol";

type LogFn = (message: string) => void;
type ProgressFn = (done: number, total: number) => void;

const logs = new Set<LogFn>();
const progress = new Set<ProgressFn>();
const pending = new Map<number, { resolve: (value: unknown) => void; reject: (err: Error) => void }>();

let worker: Worker | null = null;
let sab: SharedArrayBuffer | null = null;
let device: USBDevice | null = null;
let pumping = false;
let nextId = 1;
let readyPromise: Promise<void> | null = null;

export function onLog(fn: LogFn): () => void {
  logs.add(fn);
  return () => logs.delete(fn);
}

export function onProgress(fn: ProgressFn): () => void {
  progress.add(fn);
  return () => progress.delete(fn);
}

export function prepare(): Promise<void> {
  if (!readyPromise) {
    readyPromise = boot().catch((err) => {
      readyPromise = null;
      throw err;
    });
  }
  return readyPromise;
}

function boot(): Promise<void> {
  if (typeof SharedArrayBuffer === "undefined" || !crossOriginIsolated) {
    return Promise.reject(
      new Error(
        "This page is not cross-origin isolated, so WebUSB transfers cannot run. Start it with npm run dev:web.",
      ),
    );
  }
  sab = new SharedArrayBuffer(SAB_SIZE);
  worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
  return new Promise((resolve, reject) => {
    worker!.onerror = (event: ErrorEvent) => {
      reject(
        new Error(
          event.message || `The WebUSB worker failed (${event.filename}:${event.lineno}).`,
        ),
      );
    };
    worker!.onmessage = (event: MessageEvent) => {
      const message = event.data;
      if (message.type === "ready") {
        resolve();
        return;
      }
      if (message.type === "ready-error") {
        reject(new Error(message.error));
        return;
      }
      if (message.type === "log") {
        for (const fn of logs) fn(message.message);
        return;
      }
      if (message.type === "progress") {
        for (const fn of progress) fn(message.done, message.total);
        return;
      }
      if (message.type === "result") {
        const waiter = pending.get(message.id);
        pending.delete(message.id);
        if (!waiter) return;
        if (message.ok) waiter.resolve(message.bytes ?? message.result);
        else waiter.reject(new Error(message.error || "Command failed."));
      }
    };
    worker!.postMessage({ type: "boot", sab });
  });
}

function request<T>(message: Record<string, unknown>): Promise<T> {
  const id = nextId++;
  return new Promise((resolve, reject) => {
    pending.set(id, {
      resolve: (value) => resolve(value as T),
      reject,
    });
    worker?.postMessage({ ...message, id });
  });
}

export async function command<T>(method: string, args: Record<string, unknown> = {}): Promise<T> {
  await prepare();
  const result = await request<string>({ type: "call", method, args });
  return JSON.parse(result) as T;
}

export async function loadBytes(
  name: string,
  filename: string,
  page: number,
  model: string,
  bytes: Uint8Array,
): Promise<string> {
  await prepare();
  return request<string>({ type: "load", name, filename, page, model, bytes });
}

export async function compareBytes(filename: string, bytes: Uint8Array): Promise<string> {
  await prepare();
  return request<string>({ type: "compare", filename, bytes });
}

export async function encodeBuffer(filename: string): Promise<Uint8Array> {
  await prepare();
  return request<Uint8Array>({ type: "encode", filename });
}

export async function programmerStatus<T>(force: boolean): Promise<T> {
  if (!force) return command<T>("programmer_status", { force: false });
  if (!navigator.usb) {
    throw new Error("This browser has no WebUSB. Open the page in Chrome or Edge.");
  }
  const selected = await navigator.usb.requestDevice({ filters: PROGRAMMER_FILTERS });
  await prepare();
  await claim(selected);
  ensurePump();
  return command<T>("programmer_status", {
    force: true,
    vid: selected.vendorId,
    pid: selected.productId,
  });
}

async function claim(next: USBDevice) {
  if (device && device !== next) {
    try {
      await device.close();
    } catch {
      // The previous device may already be gone.
    }
  }
  device = next;
  if (!next.opened) await next.open();
  if (next.configuration === null) await next.selectConfiguration(1);
  try {
    await next.claimInterface(0);
  } catch (err) {
    const claimed = next.configuration?.interfaces.some(
      (item) => item.interfaceNumber === 0 && item.claimed,
    );
    if (!claimed) throw err;
  }
}

function ensurePump() {
  if (pumping) return;
  pumping = true;
  void pump();
}

async function pump() {
  if (!sab) return;
  const words = new Int32Array(sab);
  const raw = new Uint8Array(sab);
  while (pumping) {
    let state = Atomics.load(words, H_STATE);
    while (state !== STATE_REQ) {
      const wait = Atomics.waitAsync(words, H_STATE, state);
      if (wait.async) await wait.value;
      state = Atomics.load(words, H_STATE);
    }
    try {
      if (!device) throw new Error("No programmer is open.");
      await transfer(device, words, raw);
      Atomics.store(words, H_STATUS, 0);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      const encoded = new TextEncoder().encode(`USB error: ${message}`);
      const n = Math.min(encoded.length, 1024);
      raw.set(encoded.subarray(0, n), DATA_OFFSET);
      Atomics.store(words, H_ACTUAL, n);
      Atomics.store(words, H_ACTUAL_B, 0);
      Atomics.store(words, H_STATUS, 1);
    }
    Atomics.store(words, H_STATE, STATE_RESP);
    Atomics.notify(words, H_STATE);
  }
}

async function transfer(current: USBDevice, words: Int32Array, raw: Uint8Array) {
  const op = Atomics.load(words, H_OP);
  const epA = Atomics.load(words, H_EP_A) & 0x7f;
  const epB = Atomics.load(words, H_EP_B) & 0x7f;
  const lenA = Atomics.load(words, H_LEN_A);
  const lenB = Atomics.load(words, H_LEN_B);
  const incoming = op === OP_IN || op === OP_IN_PAIR;
  if (op === OP_OUT) {
    const payload = raw.slice(DATA_OFFSET, DATA_OFFSET + lenA);
    const result = await timed(current.transferOut(epA, payload), incoming);
    if (result.status !== "ok") throw new Error(`endpoint ${result.status.toUpperCase()}`);
    return;
  }
  if (op === OP_IN) {
    const result = await timed(current.transferIn(epA, lenA), incoming);
    const got = dataBytes(result);
    raw.set(got, DATA_OFFSET);
    Atomics.store(words, H_ACTUAL, got.length);
    return;
  }
  if (op === OP_OUT_PAIR) {
    const a = raw.slice(DATA_OFFSET, DATA_OFFSET + lenA);
    const b = raw.slice(DATA_OFFSET + lenA, DATA_OFFSET + lenA + lenB);
    const [left, right] = await timed(
      Promise.all([current.transferOut(epA, a), current.transferOut(epB, b)]),
      incoming,
    );
    if (left.status !== "ok" || right.status !== "ok") {
      throw new Error(`endpoint ${left.status.toUpperCase()}`);
    }
    return;
  }
  if (op === OP_IN_PAIR) {
    const [left, right] = await timed(
      Promise.all([current.transferIn(epA, lenA), current.transferIn(epB, lenB)]),
      incoming,
    );
    const bytesA = dataBytes(left);
    const bytesB = dataBytes(right);
    raw.set(bytesA, DATA_OFFSET);
    raw.set(bytesB, DATA_OFFSET + lenA);
    Atomics.store(words, H_ACTUAL, bytesA.length);
    Atomics.store(words, H_ACTUAL_B, bytesB.length);
    return;
  }
  throw new Error(`Unknown USB operation ${op}.`);
}

function dataBytes(result: USBInTransferResult): Uint8Array {
  if (result.status !== "ok" || !result.data) {
    throw new Error(`endpoint ${String(result.status).toUpperCase()}`);
  }
  return new Uint8Array(result.data.buffer, result.data.byteOffset, result.data.byteLength);
}

async function timed<T>(work: Promise<T>, incoming: boolean): Promise<T> {
  const timeoutMs = incoming ? RECV_TIMEOUT_MS : SEND_TIMEOUT_MS;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      reject(
        new Error(
          "transfer timed out — the programmer may be in a bad state. Unplug it, wait 20-30 seconds, plug it back in, and try again.",
        ),
      );
    }, timeoutMs);
  });
  try {
    return await Promise.race([work, timeout]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}

export async function pickFile(): Promise<File | null> {
  if (!("showOpenFilePicker" in window)) {
    throw new Error("This browser cannot open local files from the page.");
  }
  try {
    const [handle] = await window.showOpenFilePicker({
      multiple: false,
      types: [
        {
          description: "Firmware",
          accept: {
            "application/octet-stream": [".bin", ".hex", ".ihx", ".ihex", ".srec", ".s19", ".s28", ".s37", ".jed"],
          },
        },
      ],
    });
    return await handle.getFile();
  } catch (err) {
    if (err instanceof DOMException && err.name === "AbortError") return null;
    throw err;
  }
}

export async function saveInteractive(
  encode: (filename: string) => Promise<Uint8Array>,
): Promise<string> {
  if (!("showSaveFilePicker" in window)) {
    const bytes = await encode("dump.bin");
    download("dump.bin", bytes);
    return "dump.bin";
  }
  try {
    const handle = await window.showSaveFilePicker({
      suggestedName: "dump.bin",
      types: [
        { description: "Binary", accept: { "application/octet-stream": [".bin"] } },
        { description: "Intel HEX", accept: { "text/plain": [".hex"] } },
        { description: "S-record", accept: { "text/plain": [".srec"] } },
      ],
    });
    const bytes = await encode(handle.name);
    const writable = await handle.createWritable();
    await writable.write(new Uint8Array(bytes));
    await writable.close();
    return handle.name;
  } catch (err) {
    if (err instanceof DOMException && err.name === "AbortError") return "";
    throw err;
  }
}

function download(filename: string, bytes: Uint8Array) {
  const blob = new Blob([Uint8Array.from(bytes)]);
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.click();
  URL.revokeObjectURL(url);
}
