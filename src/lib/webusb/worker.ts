import init, {
  compare_bytes,
  dispatch,
  encode_buffer,
  load_file_bytes,
  mount_databases,
  usb_memory,
} from "./pkg/minipro_web.js";
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
  MAX_BYTES,
  OP_IN,
  OP_IN_PAIR,
  OP_OUT,
  OP_OUT_PAIR,
  STATE_IDLE,
  STATE_REQ,
  STATE_RESP,
} from "./protocol";

let sab: SharedArrayBuffer | null = null;
let lastError = "";

function memory(): WebAssembly.Memory {
  const mem = (globalThis as { miniproMemory?: WebAssembly.Memory }).miniproMemory;
  if (!mem) throw new Error("WebAssembly memory is not ready.");
  return mem;
}

function wasmBytes(ptr: number, len: number): Uint8Array {
  return new Uint8Array(memory().buffer, ptr, len);
}

function header(): Int32Array {
  if (!sab) throw new Error("USB transport is not ready.");
  return new Int32Array(sab);
}

function bytes(): Uint8Array {
  if (!sab) throw new Error("USB transport is not ready.");
  return new Uint8Array(sab);
}

function submit(
  op: number,
  epA: number,
  epB: number,
  lenA: number,
  lenB: number,
  payload: Uint8Array | null,
): { status: number; actual: number; actualB: number; data: Uint8Array } {
  if (lenA + lenB > MAX_BYTES) {
    throw new Error(`USB transfer of ${lenA + lenB} bytes exceeds the WebUSB buffer.`);
  }
  const words = header();
  const raw = bytes();
  if (Atomics.load(words, H_STATE) !== STATE_IDLE) {
    throw new Error("USB transport is busy.");
  }
  if (payload) raw.set(payload, DATA_OFFSET);
  Atomics.store(words, H_OP, op);
  Atomics.store(words, H_EP_A, epA);
  Atomics.store(words, H_EP_B, epB);
  Atomics.store(words, H_LEN_A, lenA);
  Atomics.store(words, H_LEN_B, lenB);
  Atomics.store(words, H_STATUS, 0);
  Atomics.store(words, H_ACTUAL, 0);
  Atomics.store(words, H_ACTUAL_B, 0);
  Atomics.store(words, H_STATE, STATE_REQ);
  Atomics.notify(words, H_STATE);
  while (Atomics.load(words, H_STATE) !== STATE_RESP) {
    const wait = Atomics.wait(words, H_STATE, STATE_REQ, 400_000);
    if (wait === "timed-out") {
      Atomics.store(words, H_STATE, STATE_IDLE);
      Atomics.notify(words, H_STATE);
      throw new Error(
        "USB transfer timed out — the programmer may be in a bad state. Unplug it, wait 20-30 seconds, plug it back in, and try again.",
      );
    }
  }
  const status = Atomics.load(words, H_STATUS);
  const actual = Atomics.load(words, H_ACTUAL);
  const actualB = Atomics.load(words, H_ACTUAL_B);
  const data = raw.slice(DATA_OFFSET, DATA_OFFSET + Math.max(0, actual));
  const dataB =
    actualB > 0 ? raw.slice(DATA_OFFSET + lenA, DATA_OFFSET + lenA + actualB) : new Uint8Array();
  Atomics.store(words, H_STATE, STATE_IDLE);
  Atomics.notify(words, H_STATE);
  return { status, actual, actualB, data: actualB > 0 ? concat(data, dataB) : data };
}

function concat(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(a.length + b.length);
  out.set(a, 0);
  out.set(b, a.length);
  return out;
}

function fail(err: unknown): number {
  lastError = err instanceof Error ? err.message : String(err);
  return -1;
}

function check(result: { status: number; data: Uint8Array }): number {
  if (result.status !== 0) {
    lastError = new TextDecoder().decode(result.data) || "USB transfer failed.";
    return -1;
  }
  return 0;
}

(globalThis as { miniproHostLog?: (message: string) => void }).miniproHostLog = (message) => {
  self.postMessage({ type: "log", message });
};

(globalThis as { miniproHostProgress?: (done: number, total: number) => void }).miniproHostProgress =
  (done, total) => {
    self.postMessage({ type: "progress", done, total });
  };

(globalThis as { miniproHostError?: () => string }).miniproHostError = () => lastError;

(globalThis as { miniproHostOut?: (ep: number, ptr: number, len: number) => number }).miniproHostOut =
  (ep, ptr, len) => {
    try {
      const payload = wasmBytes(ptr, len).slice();
      return check(submit(OP_OUT, ep, 0, len, 0, payload));
    } catch (err) {
      return fail(err);
    }
  };

(globalThis as { miniproHostIn?: (ep: number, ptr: number, len: number) => number }).miniproHostIn =
  (ep, ptr, len) => {
    try {
      const result = submit(OP_IN, ep, 0, len, 0, null);
      if (check(result) < 0) return -1;
      new Uint8Array(memory().buffer, ptr, result.data.length).set(result.data);
      return result.data.length;
    } catch (err) {
      return fail(err);
    }
  };

(
  globalThis as {
    miniproHostOutPair?: (
      epA: number,
      ptrA: number,
      lenA: number,
      epB: number,
      ptrB: number,
      lenB: number,
    ) => number;
  }
).miniproHostOutPair = (epA, ptrA, lenA, epB, ptrB, lenB) => {
  try {
    const payload = new Uint8Array(lenA + lenB);
    payload.set(wasmBytes(ptrA, lenA), 0);
    payload.set(wasmBytes(ptrB, lenB), lenA);
    return check(submit(OP_OUT_PAIR, epA, epB, lenA, lenB, payload));
  } catch (err) {
    return fail(err);
  }
};

(
  globalThis as {
    miniproHostInPair?: (
      epA: number,
      ptrA: number,
      lenA: number,
      epB: number,
      ptrB: number,
      lenB: number,
    ) => number;
  }
).miniproHostInPair = (epA, ptrA, lenA, epB, ptrB, lenB) => {
  try {
    const result = submit(OP_IN_PAIR, epA, epB, lenA, lenB, null);
    if (check(result) < 0) return -1;
    if (result.actual !== lenA || result.actualB !== lenB) {
      throw new Error(
        `USB error: short paired read, got ${result.actual} and ${result.actualB} bytes, expected ${lenA} and ${lenB}`,
      );
    }
    const mem = new Uint8Array(memory().buffer);
    mem.set(result.data.subarray(0, lenA), ptrA);
    mem.set(result.data.subarray(lenA, lenA + lenB), ptrB);
    return 0;
  } catch (err) {
    return fail(err);
  }
};

function reply(id: number, ok: boolean, result?: unknown, error?: string, bytes?: Uint8Array) {
  self.postMessage({ type: "result", id, ok, result, error, bytes });
}

async function boot() {
  self.postMessage({ type: "log", message: "Loading the chip database…" });
  await init();
  (globalThis as { miniproMemory?: WebAssembly.Memory }).miniproMemory =
    usb_memory() as WebAssembly.Memory;
  const infoic = await fetch("/db/infoic.xml");
  const logicic = await fetch("/db/logicic.xml");
  if (!infoic.ok || !logicic.ok) {
    throw new Error("Could not load the chip database.");
  }
  const algorithms = await fetch("/db/algorithm.xml");
  const infoicText = await infoic.text();
  const logicicText = await logicic.text();
  self.postMessage({
    type: "log",
    message: `Mounting ${infoicText.length.toLocaleString()} bytes of device data…`,
  });
  mount_databases(infoicText, logicicText, algorithms.ok ? await algorithms.text() : null);
  self.postMessage({ type: "log", message: "Chip database ready." });
  self.postMessage({ type: "ready" });
}

self.onmessage = (event: MessageEvent) => {
  const message = event.data;
  if (message.type === "boot") {
    sab = message.sab as SharedArrayBuffer;
    boot().catch((err: unknown) => {
      self.postMessage({
        type: "ready-error",
        error: err instanceof Error ? err.message : String(err),
      });
    });
    return;
  }
  const id = message.id as number;
  try {
    if (message.type === "call") {
      reply(id, true, dispatch(message.method, JSON.stringify(message.args ?? {})));
      return;
    }
    if (message.type === "load") {
      reply(
        id,
        true,
        load_file_bytes(message.name, message.filename, message.page, message.model, message.bytes),
      );
      return;
    }
    if (message.type === "compare") {
      reply(id, true, compare_bytes(message.filename, message.bytes));
      return;
    }
    if (message.type === "encode") {
      reply(id, true, undefined, undefined, encode_buffer(message.filename));
      return;
    }
    reply(id, false, undefined, "Unknown worker message.");
  } catch (err) {
    reply(id, false, undefined, err instanceof Error ? err.message : String(err));
  }
};
