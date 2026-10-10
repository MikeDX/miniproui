import { isTauri, invoke } from "@tauri-apps/api/core";
import * as web from "$lib/webusb/client";

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
  kind: string;
  fuses: FuseFieldInfo[];
}

export interface FuseFieldInfo {
  name: string;
  mask: number;
  defaultValue: number;
  group: string;
}

export interface FuseReading {
  name: string;
  value: number;
  mask: number;
  group: string;
}

export interface LogicReport {
  pass: boolean;
  errors: number;
  pinCount: number;
  vectorCount: number;
  message: string;
}

export interface PinReport {
  message: string;
  badPins: number[];
}

export interface SpiReport {
  jedecId: string;
  matches: SearchHit[];
}

export interface FileReport {
  matches: boolean;
  message: string;
  bufferLength: number;
  memoryBytes: number;
  fileBytes: number;
  crc32: string;
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

function command<T>(name: string, args: Record<string, unknown> = {}): Promise<T> {
  if (isTauri()) return invoke<T>(name, args);
  return web.command<T>(name, args);
}

export function prepareBrowser(): Promise<void> {
  if (isTauri()) return Promise.resolve();
  return web.prepare();
}

export function onBrowserLog(fn: (message: string) => void): () => void {
  if (isTauri()) return () => {};
  return web.onLog(fn);
}

export function onBrowserProgress(fn: (done: number, total: number) => void): () => void {
  if (isTauri()) return () => {};
  return web.onProgress(fn);
}

export function pickBrowserFile(): Promise<File | null> {
  return web.pickFile();
}

export function saveBrowser(encode: (filename: string) => Promise<Uint8Array>): Promise<string> {
  return web.saveInteractive(encode);
}

export function encodeBrowser(filename: string): Promise<Uint8Array> {
  return web.encodeBuffer(filename);
}

export async function loadBrowserFile(
  name: string,
  file: File,
  page: number,
  model: string,
): Promise<FileReport> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  const text = await web.loadBytes(name, file.name, page, model, bytes);
  return JSON.parse(text) as FileReport;
}

export async function compareBrowserFile(file: File): Promise<DumpReport> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  const text = await web.compareBytes(file.name, bytes);
  return JSON.parse(text) as DumpReport;
}

export function errorText(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err && typeof err.message === "string") {
    return err.message;
  }
  return String(err);
}

export function programmerStatus(force: boolean): Promise<ProgrammerStatus> {
  if (!isTauri() && force) return web.programmerStatus<ProgrammerStatus>(true);
  return command("programmer_status", { force });
}

export function searchDevices(query: string, model: string): Promise<SearchResponse> {
  return command("search_devices", { query, model });
}

export function deviceInfo(name: string, model: string): Promise<DeviceInfo> {
  return command("device_info", { name, model });
}

export function loadFile(
  path: string,
  device: string,
  page: number,
  model: string,
): Promise<FileReport> {
  return command("load_file", { path, device, page, model });
}

export function hexWindow(offset: number, length: number): Promise<HexWindow> {
  return command("hex_window", { offset, length });
}

export function saveBuffer(path: string): Promise<string> {
  return command("save_buffer", { path });
}

export function readMemory(device: string, page: number): Promise<OpReport> {
  return command("read_memory", { device, page });
}

export function writeFile(
  device: string,
  page: number,
  path: string,
  confirm: boolean,
): Promise<OpReport> {
  return command("write_file", { device, page, path, confirm });
}

export function verifyFile(device: string, page: number, path: string): Promise<OpReport> {
  return command("verify_file", { device, page, path });
}

export function eraseChip(device: string, confirm: boolean): Promise<OpReport> {
  return command("erase", { device, confirm });
}

export function blankCheck(device: string): Promise<OpReport> {
  return command("blank_check", { device });
}

export function readId(device: string): Promise<OpReport> {
  return command("read_id", { device });
}

export function setOptions(skipId: boolean, icsp: string): Promise<void> {
  return command("set_options", { skipId, icsp });
}

export function patchBuffer(offset: number, bytes: number[]): Promise<string> {
  return command("patch_buffer", { offset, bytes });
}

export function writeBuffer(device: string, page: number, confirm: boolean): Promise<OpReport> {
  return command("write_buffer", { device, page, confirm });
}

export function verifyBuffer(device: string, page: number): Promise<OpReport> {
  return command("verify_buffer", { device, page });
}

export function readConfig(device: string): Promise<FuseReading[]> {
  return command("read_config", { device });
}

export function writeConfig(
  device: string,
  fuses: { name: string; value: number }[],
  confirm: boolean,
): Promise<OpReport> {
  return command("write_config", { device, fuses, confirm });
}

export function logicTest(device: string): Promise<LogicReport> {
  return command("logic_test", { device });
}

export function pinCheck(device: string): Promise<PinReport> {
  return command("pin_check", { device });
}

export function hardwareTest(): Promise<OpReport> {
  return command("hardware_test");
}

export function spiDetect(pins: number): Promise<SpiReport> {
  return command("spi_detect", { pins });
}

export function updateFirmware(path: string, confirm: boolean): Promise<OpReport> {
  return command("update_firmware", { path, confirm });
}

export interface DumpReport {
  crc32: string;
  sha1: string;
  length: number;
  lines: string[];
}

export interface RomIdentReport {
  lines: string[];
}

export function bufferReport(): Promise<DumpReport> {
  return command("buffer_report");
}

export function romIdent(): Promise<RomIdentReport> {
  return command("rom_ident");
}

export function compareBuffer(path: string): Promise<DumpReport> {
  return command("compare_buffer", { path });
}
