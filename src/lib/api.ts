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

export function setOptions(skipId: boolean, icsp: string): Promise<void> {
  return invoke("set_options", { skipId, icsp });
}

export function patchBuffer(offset: number, bytes: number[]): Promise<string> {
  return invoke("patch_buffer", { offset, bytes });
}

export function writeBuffer(device: string, page: number, confirm: boolean): Promise<OpReport> {
  return invoke("write_buffer", { device, page, confirm });
}

export function verifyBuffer(device: string, page: number): Promise<OpReport> {
  return invoke("verify_buffer", { device, page });
}

export function readConfig(device: string): Promise<FuseReading[]> {
  return invoke("read_config", { device });
}

export function writeConfig(
  device: string,
  fuses: { name: string; value: number }[],
  confirm: boolean,
): Promise<OpReport> {
  return invoke("write_config", { device, fuses, confirm });
}

export function logicTest(device: string): Promise<LogicReport> {
  return invoke("logic_test", { device });
}

export function pinCheck(device: string): Promise<PinReport> {
  return invoke("pin_check", { device });
}

export function hardwareTest(): Promise<OpReport> {
  return invoke("hardware_test");
}

export function spiDetect(pins: number): Promise<SpiReport> {
  return invoke("spi_detect", { pins });
}

export function updateFirmware(path: string, confirm: boolean): Promise<OpReport> {
  return invoke("update_firmware", { path, confirm });
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
  return invoke("buffer_report");
}

export function romIdent(): Promise<RomIdentReport> {
  return invoke("rom_ident");
}

export function compareBuffer(path: string): Promise<DumpReport> {
  return invoke("compare_buffer", { path });
}
