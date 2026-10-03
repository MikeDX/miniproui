import { invoke } from "@tauri-apps/api/core";

export interface ProgrammerStatus {
  connected: boolean;
  busy: boolean;
  model: string | null;
  firmware: string | null;
  serialNumber: string | null;
  message: string;
  warning: string | null;
}

export interface SearchHit {
  name: string;
  manufacturer: string;
}

export interface SearchResponse {
  hits: SearchHit[];
  truncated: boolean;
}

export interface DeviceInfo {
  name: string;
  manufacturer: string;
  package: string;
  codeBytes: number;
  dataBytes: number;
  chipId: string;
  hasChipId: boolean;
  blankValue: string;
  canErase: boolean;
}

export interface FileReport {
  matches: boolean;
  message: string;
  bufferLength: number;
  memoryBytes: number;
  fileBytes: number;
}

export interface OpReport {
  message: string;
  bytes: number;
  crc32: string | null;
  chipId: string | null;
  bufferLength: number;
}

export interface HexWindow {
  offset: number;
  length: number;
  bytes: number[];
}

export interface ProgressEvent {
  done: number;
  total: number;
}

export interface LogEvent {
  message: string;
}

export function errorText(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err && typeof err.message === "string") {
    return err.message;
  }
  return String(err);
}

export function programmerStatus(force: boolean): Promise<ProgrammerStatus> {
  return invoke("programmer_status", { force });
}

export function searchDevices(query: string, model: string): Promise<SearchResponse> {
  return invoke("search_devices", { query, model });
}

export function deviceInfo(name: string, model: string): Promise<DeviceInfo> {
  return invoke("device_info", { name, model });
}

export function loadFile(
  path: string,
  device: string,
  page: number,
  model: string,
): Promise<FileReport> {
  return invoke("load_file", { path, device, page, model });
}

export function hexWindow(offset: number, length: number): Promise<HexWindow> {
  return invoke("hex_window", { offset, length });
}

export function saveBuffer(path: string): Promise<string> {
  return invoke("save_buffer", { path });
}

export function readMemory(device: string, page: number): Promise<OpReport> {
  return invoke("read_memory", { device, page });
}

export function writeFile(
  device: string,
  page: number,
  path: string,
  confirm: boolean,
): Promise<OpReport> {
  return invoke("write_file", { device, page, path, confirm });
}

export function verifyFile(device: string, page: number, path: string): Promise<OpReport> {
  return invoke("verify_file", { device, page, path });
}

export function eraseChip(device: string, confirm: boolean): Promise<OpReport> {
  return invoke("erase", { device, confirm });
}

export function blankCheck(device: string): Promise<OpReport> {
  return invoke("blank_check", { device });
}

export function readId(device: string): Promise<OpReport> {
  return invoke("read_id", { device });
}
