export function onLog(_fn: (message: string) => void): () => void {
  return () => {};
}

export function onProgress(_fn: (done: number, total: number) => void): () => void {
  return () => {};
}

export function prepare(): Promise<void> {
  return Promise.resolve();
}

export function command<T>(_method: string, _args?: Record<string, unknown>): Promise<T> {
  return Promise.reject(new Error("WebUSB is not part of the desktop build."));
}

export function loadBytes(): Promise<string> {
  return Promise.reject(new Error("WebUSB is not part of the desktop build."));
}

export function compareBytes(): Promise<string> {
  return Promise.reject(new Error("WebUSB is not part of the desktop build."));
}

export function encodeBuffer(): Promise<Uint8Array> {
  return Promise.reject(new Error("WebUSB is not part of the desktop build."));
}

export function programmerStatus<T>(): Promise<T> {
  return Promise.reject(new Error("WebUSB is not part of the desktop build."));
}

export function pickFile(): Promise<File | null> {
  return Promise.reject(new Error("WebUSB is not part of the desktop build."));
}

export function saveInteractive(): Promise<string> {
  return Promise.reject(new Error("WebUSB is not part of the desktop build."));
}
