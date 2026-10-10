//! Programmer session, chip-database search, and chip operations.
//!
//! USB work is synchronous. Callers run these methods on a blocking thread.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use minipro_core::database::{find_device, list_devices_for_model, DatabasePaths, DeviceListItem};
use minipro_core::device::{ChipConfig, Device, ProgrammerModel, ICSP_ENABLE, ICSP_VCC};
use minipro_core::error::MiniproError;
use minipro_core::handle::MiniproHandle;
use minipro_core::operations::{
    blank_check, erase_chip, firmware_update, logic_ic_test, pin_contact_check, read_chip,
    read_file, read_fuses, spi_autodetect_and_lookup, verify_chip_bytes, write_chip_bytes,
    write_file, write_fuses, FuseValue, SizeMismatch,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

const SEARCH_LIMIT: usize = 200;
const HEX_WINDOW_MAX: u32 = 8192;

#[derive(Clone)]
pub struct AppState {
    pub db: DatabasePaths,
    session: Arc<Mutex<Session>>,
    index: Arc<Mutex<HashMap<ProgrammerModel, Vec<DeviceListItem>>>>,
}

struct Session {
    handle: Option<MiniproHandle>,
    busy: bool,
    last_info: Option<LastInfo>,
    buffer_path: PathBuf,
    buffer_len: u64,
    skip_id: bool,
    icsp: u8,
}

struct LastInfo {
    model: String,
    firmware: String,
    serial_number: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgrammerStatusDto {
    pub connected: bool,
    pub busy: bool,
    pub model: Option<String>,
    pub firmware: Option<String>,
    pub serial_number: Option<String>,
    pub message: String,
    pub warning: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub name: String,
    pub manufacturer: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub name: String,
    pub manufacturer: String,
    pub package: String,
    pub code_bytes: u32,
    pub data_bytes: u32,
    pub chip_id: String,
    pub has_chip_id: bool,
    pub blank_value: String,
    pub can_erase: bool,
    pub kind: String,
    pub fuses: Vec<FuseFieldInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FuseFieldInfo {
    pub name: String,
    pub mask: u16,
    pub default_value: u16,
    pub group: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FuseWrite {
    pub name: String,
    pub value: u16,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FuseReading {
    pub name: String,
    pub value: u16,
    pub mask: u16,
    pub group: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogicReport {
    pub pass: bool,
    pub errors: u32,
    pub pin_count: u16,
    pub vector_count: u16,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinReport {
    pub message: String,
    pub bad_pins: Vec<u16>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpiReport {
    pub jedec_id: String,
    pub matches: Vec<SearchHit>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileReport {
    pub matches: bool,
    pub message: String,
    pub buffer_length: u64,
    pub memory_bytes: u64,
    pub file_bytes: u64,
    pub crc32: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpReport {
    pub message: String,
    pub bytes: u64,
    pub crc32: Option<String>,
    pub chip_id: Option<String>,
    pub buffer_length: u64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HexWindow {
    pub offset: u64,
    pub length: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Serialize)]
struct LogEvent {
    message: String,
}

#[derive(Clone, Serialize)]
struct ProgressEvent {
    done: u64,
    total: u64,
}

enum Fail {
    Soft(String),
    Fatal(String),
}

struct BusyGuard<'a> {
    session: &'a Mutex<Session>,
    handle: Option<MiniproHandle>,
    fatal: bool,
}

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        let Some(handle) = self.handle.take() else {
            return;
        };
        let mut session = lock(self.session);
        session.busy = false;
        if self.fatal {
            session.handle = None;
            session.last_info = None;
        } else {
            session.handle = Some(handle);
        }
    }
}

impl AppState {
    pub fn new() -> Result<Self, String> {
        let db = database_paths()?;
        let dir = std::env::temp_dir().join("miniproui");
        fs::create_dir_all(&dir).map_err(|e| format!("cannot create temp dir: {e}"))?;
        static BUFFER_SEQ: AtomicU64 = AtomicU64::new(0);
        let id = BUFFER_SEQ.fetch_add(1, Ordering::Relaxed);
        let buffer_path = dir.join(format!("buffer-{}-{id}.bin", std::process::id()));
        if !buffer_path.exists() {
            fs::write(&buffer_path, []).map_err(|e| format!("cannot create buffer file: {e}"))?;
        }
        Ok(Self {
            db,
            session: Arc::new(Mutex::new(Session {
                handle: None,
                busy: false,
                last_info: None,
                buffer_path,
                buffer_len: 0,
                skip_id: false,
                icsp: 0,
            })),
            index: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn refresh_programmer(&self, force: bool) -> ProgrammerStatusDto {
        {
            let mut session = lock(&self.session);
            if session.busy {
                return status_from(&session, true, None);
            }
            if !force && session.handle.is_some() {
                let warning = session
                    .handle
                    .as_ref()
                    .and_then(|handle| algorithm_warning(handle.info.model, &self.db));
                return status_from(&session, false, warning);
            }
            session.busy = true;
            if force {
                session.handle = None;
            }
        }

        let opened = MiniproHandle::open().map(|mut handle| {
            handle.db_paths = Some(self.db.clone());
            handle
        });

        let mut session = lock(&self.session);
        session.busy = false;
        match opened {
            Ok(handle) => {
                let warning = algorithm_warning(handle.info.model, &self.db);
                session.last_info = Some(LastInfo {
                    model: handle.info.model.to_string(),
                    firmware: handle.info.firmware_str.clone(),
                    serial_number: handle.info.serial_number.clone(),
                });
                session.handle = Some(handle);
                status_from(&session, false, warning)
            }
            Err(err) => {
                session.handle = None;
                session.last_info = None;
                disconnected(err)
            }
        }
    }

    pub fn search(&self, query: &str, model: &str) -> Result<SearchResponse, String> {
        let query = query.trim();
        if query.len() < 2 {
            return Ok(SearchResponse {
                hits: Vec::new(),
                truncated: false,
            });
        }
        let model = parse_model(model)?;
        self.ensure_index(model)?;
        let index = lock(&self.index);
        let items = index
            .get(&model)
            .ok_or_else(|| "device index missing".to_string())?;
        let needle = query.to_ascii_lowercase();
        let mut hits = Vec::new();
        let mut truncated = false;
        for item in items {
            if !item.name.to_ascii_lowercase().contains(&needle) {
                continue;
            }
            if hits.len() == SEARCH_LIMIT {
                truncated = true;
                break;
            }
            hits.push(SearchHit {
                name: item.name.clone(),
                manufacturer: item.manufacturer.clone(),
            });
        }
        Ok(SearchResponse { hits, truncated })
    }

    pub fn device_info(&self, name: &str, model: &str) -> Result<DeviceInfo, String> {
        let model = parse_model(model)?;
        let device = find_device(&self.db, name, model).map_err(|e| e.to_string())?;
        Ok(device_info(name, &device))
    }

    pub fn load_file(
        &self,
        path: &Path,
        device_name: Option<&str>,
        page: u8,
        model: &str,
    ) -> Result<FileReport, String> {
        if lock(&self.session).busy {
            return Err("Another operation is already running.".into());
        }
        let file_bytes = fs::metadata(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?
            .len();
        if file_bytes == 0 {
            return Err("File is empty.".into());
        }
        let format = format_for(path);

        let Some(device_name) = device_name.filter(|n| !n.is_empty()) else {
            if format != "bin" {
                return Err("Select a device before loading an Intel HEX or S-record file.".into());
            }
            let bytes = fs::read(path).map_err(|e| e.to_string())?;
            self.store_buffer(&bytes)?;
            let len = bytes.len() as u64;
            return Ok(FileReport {
                matches: true,
                message: format!("Loaded {len} bytes."),
                buffer_length: len,
                memory_bytes: 0,
                file_bytes,
                crc32: crc_hex(&bytes),
            });
        };

        let model = parse_model(model)?;
        let device = find_device(&self.db, device_name, model).map_err(|e| e.to_string())?;
        let memory = page_size(&device, page)?;
        if format == "bin" {
            let bytes = fs::read(path).map_err(|e| e.to_string())?;
            let matches = bytes.len() == memory;
            self.store_buffer(&bytes)?;
            let message = if matches {
                format!("Loaded {} bytes.", bytes.len())
            } else {
                format!(
                    "File is {} bytes, but this memory is {memory} bytes.",
                    bytes.len()
                )
            };
            return Ok(FileReport {
                matches,
                message,
                buffer_length: bytes.len() as u64,
                memory_bytes: memory as u64,
                file_bytes,
                crc32: crc_hex(&bytes),
            });
        }

        let decoded =
            read_file(path, format, memory, device.blank_value as u8).map_err(|e| e.to_string())?;
        let matches = decoded.len() == memory;
        self.store_buffer(&decoded)?;
        Ok(FileReport {
            matches,
            message: format!("Decoded {format} into {} bytes.", decoded.len()),
            buffer_length: decoded.len() as u64,
            memory_bytes: memory as u64,
            file_bytes,
            crc32: crc_hex(&decoded),
        })
    }

    pub fn hex_window(&self, offset: u64, length: u32) -> Result<HexWindow, String> {
        let length = length.min(HEX_WINDOW_MAX);
        let (path, total) = {
            let session = lock(&self.session);
            (session.buffer_path.clone(), session.buffer_len)
        };
        if length == 0 || total == 0 || offset >= total {
            return Ok(HexWindow {
                offset,
                length: total,
                bytes: Vec::new(),
            });
        }
        let want = ((total - offset) as u32).min(length) as usize;
        let mut file = File::open(&path).map_err(|e| format!("cannot open buffer: {e}"))?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| format!("cannot seek buffer: {e}"))?;
        let mut bytes = vec![0u8; want];
        let n = file
            .read(&mut bytes)
            .map_err(|e| format!("cannot read buffer: {e}"))?;
        bytes.truncate(n);
        Ok(HexWindow {
            offset,
            length: total,
            bytes,
        })
    }

    pub fn save_buffer(&self, path: &Path) -> Result<String, String> {
        let (src, len) = {
            let session = lock(&self.session);
            (session.buffer_path.clone(), session.buffer_len)
        };
        if len == 0 {
            return Err("Nothing to save.".into());
        }
        let data = fs::read(&src).map_err(|e| format!("cannot read buffer: {e}"))?;
        let format = format_for(path);
        write_file(path, format, &data, None).map_err(|e| e.to_string())?;
        Ok(format!("Saved {} bytes to {}.", data.len(), path.display()))
    }

    pub fn read_memory(&self, app: &AppHandle, name: &str, page: u8) -> Result<OpReport, String> {
        let mut guard = self.busy_guard()?;
        let handle = guard.handle.as_mut().expect("handle");
        let result = (|| {
            let check_id = self.prepare(handle);
            let device = lookup(&self.db, handle, name)?;
            let memory = page_size(&device, page).map_err(Fail::Soft)?;
            log_line(
                app,
                format!("Reading {memory} bytes of {name} ({})...", page_name(page)),
            );
            let path = lock(&self.session).buffer_path.clone();
            let app = app.clone();
            let stats = transaction(handle, device, |handle| {
                let mut progress = make_progress(app.clone());
                read_chip(handle, &path, page, "bin", check_id, Some(&mut progress))
            })
            .map_err(classify)?;
            lock(&self.session).buffer_len = stats.bytes as u64;
            let _ = app.emit(
                "progress",
                ProgressEvent {
                    done: stats.bytes as u64,
                    total: stats.bytes as u64,
                },
            );
            let message = format!("Read {} bytes, CRC32 {:08x}.", stats.bytes, stats.crc32);
            log_line(&app, &message);
            Ok(report(
                message,
                stats.bytes as u64,
                Some(stats.crc32),
                None,
                stats.bytes as u64,
            ))
        })();
        guard.fatal = is_fatal(&result);
        result.map_err(fail_message)
    }

    pub fn write_memory(
        &self,
        app: &AppHandle,
        name: &str,
        page: u8,
        path: &Path,
        confirm: bool,
    ) -> Result<OpReport, String> {
        let mut guard = self.busy_guard()?;
        let handle = guard.handle.as_mut().expect("handle");
        let result = (|| {
            if !confirm {
                return Err(Fail::Soft("Write requires confirmation.".into()));
            }
            let check_id = self.prepare(handle);
            let device = lookup(&self.db, handle, name)?;
            let image = image_for(path, &device, page).map_err(Fail::Soft)?;
            let len = image.len();
            log_line(
                app,
                format!("Writing {len} bytes to {name} ({})...", page_name(page)),
            );
            let app = app.clone();
            let stats = transaction(handle, device, |handle| {
                let mut progress = make_progress(app.clone());
                write_chip_bytes(
                    handle,
                    image,
                    page,
                    SizeMismatch::Error,
                    false,
                    check_id,
                    Some(&mut progress),
                )
            })
            .map_err(classify)?;
            let _ = app.emit(
                "progress",
                ProgressEvent {
                    done: stats.bytes as u64,
                    total: stats.bytes as u64,
                },
            );
            let message = format!("Wrote {} bytes, CRC32 {:08x}.", stats.bytes, stats.crc32);
            log_line(&app, &message);
            Ok(report(
                message,
                stats.bytes as u64,
                Some(stats.crc32),
                None,
                buffer_len(&self.session),
            ))
        })();
        guard.fatal = is_fatal(&result);
        result.map_err(fail_message)
    }

    pub fn verify_memory(
        &self,
        app: &AppHandle,
        name: &str,
        page: u8,
        path: &Path,
    ) -> Result<OpReport, String> {
        let mut guard = self.busy_guard()?;
        let handle = guard.handle.as_mut().expect("handle");
        let result = (|| {
            let check_id = self.prepare(handle);
            let device = lookup(&self.db, handle, name)?;
            let image = image_for(path, &device, page).map_err(Fail::Soft)?;
            let len = image.len() as u64;
            log_line(
                app,
                format!("Verifying {len} bytes of {name} ({})...", page_name(page)),
            );
            let app = app.clone();
            transaction(handle, device, |handle| {
                let mut progress = make_progress(app.clone());
                verify_chip_bytes(handle, image, page, check_id, Some(&mut progress))
            })
            .map_err(classify)?;
            let _ = app.emit(
                "progress",
                ProgressEvent {
                    done: len,
                    total: len,
                },
            );
            let message = format!("Verify passed ({len} bytes).");
            log_line(&app, &message);
            Ok(report(message, len, None, None, buffer_len(&self.session)))
        })();
        guard.fatal = is_fatal(&result);
        result.map_err(fail_message)
    }

    pub fn erase(&self, app: &AppHandle, name: &str, confirm: bool) -> Result<OpReport, String> {
        let mut guard = self.busy_guard()?;
        let handle = guard.handle.as_mut().expect("handle");
        let result = (|| {
            if !confirm {
                return Err(Fail::Soft("Erase requires confirmation.".into()));
            }
            let check_id = self.prepare(handle);
            let device = lookup(&self.db, handle, name)?;
            if !device.flags.can_erase {
                return Err(Fail::Soft(
                    "This device cannot be electrically erased.".into(),
                ));
            }
            log_line(app, format!("Erasing {name}..."));
            transaction(handle, device, |handle| erase_chip(handle, check_id)).map_err(classify)?;
            let message = format!("Erased {name}.");
            log_line(&app, &message);
            Ok(report(message, 0, None, None, buffer_len(&self.session)))
        })();
        guard.fatal = is_fatal(&result);
        result.map_err(fail_message)
    }

    pub fn blank_check(&self, app: &AppHandle, name: &str) -> Result<OpReport, String> {
        let mut guard = self.busy_guard()?;
        let handle = guard.handle.as_mut().expect("handle");
        let result = (|| {
            self.prepare(handle);
            let device = lookup(&self.db, handle, name)?;
            log_line(app, format!("Blank-checking {name}..."));
            transaction(handle, device, |handle| blank_check(handle)).map_err(classify)?;
            let message = format!("{name} is blank.");
            log_line(&app, &message);
            Ok(report(message, 0, None, None, buffer_len(&self.session)))
        })();
        guard.fatal = is_fatal(&result);
        result.map_err(fail_message)
    }

    pub fn read_id(&self, app: &AppHandle, name: &str) -> Result<OpReport, String> {
        let mut guard = self.busy_guard()?;
        let handle = guard.handle.as_mut().expect("handle");
        let result = (|| {
            self.prepare(handle);
            let device = lookup(&self.db, handle, name)?;
            if !device.flags.has_chip_id {
                return Err(Fail::Soft("This device does not support chip ID.".into()));
            }
            let expected = device.chip_id;
            log_line(app, format!("Reading chip ID for {name}..."));
            let actual = transaction(handle, device, |handle| {
                let current = handle.device()?.clone();
                let (_kind, id) = handle.protocol.get_chip_id(&handle.usb, &current)?;
                Ok(id)
            })
            .map_err(classify)?;
            let expected_text = if expected == 0 {
                "database has no expected ID".to_string()
            } else {
                format!("database {expected:#010x}")
            };
            let message = format!("Chip ID {actual:#010x} ({expected_text}).");
            log_line(&app, &message);
            Ok(report(
                message,
                0,
                None,
                Some(format!("{actual:#010x}")),
                buffer_len(&self.session),
            ))
        })();
        guard.fatal = is_fatal(&result);
        result.map_err(fail_message)
    }

    pub fn set_options(&self, skip_id: bool, icsp: &str) -> Result<(), String> {
        let icsp = match icsp {
            "off" | "zif" => 0,
            "no-vcc" => ICSP_ENABLE,
            "vcc" => ICSP_ENABLE | ICSP_VCC,
            other => return Err(format!("Unknown socket mode '{other}'.")),
        };
        let mut session = lock(&self.session);
        session.skip_id = skip_id;
        session.icsp = icsp;
        if let Some(handle) = session.handle.as_mut() {
            handle.icsp = icsp;
        }
        Ok(())
    }

    pub fn patch_buffer(&self, offset: u64, bytes: &[u8]) -> Result<String, String> {
        let mut session = lock(&self.session);
        if session.busy {
            return Err("Another operation is already running.".into());
        }
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| "Patch is outside the buffer.".to_string())?;
        if end > session.buffer_len {
            return Err("Patch is outside the buffer.".into());
        }
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&session.buffer_path)
            .map_err(|err| format!("cannot open buffer: {err}"))?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|err| format!("cannot seek buffer: {err}"))?;
        use std::io::Write;
        file.write_all(bytes)
            .map_err(|err| format!("cannot write buffer: {err}"))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|err| format!("cannot rewind buffer: {err}"))?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)
            .map_err(|err| format!("cannot read buffer: {err}"))?;
        session.buffer_len = data.len() as u64;
        Ok(crc_hex(&data))
    }

    pub fn buffer_report(&self) -> Result<crate::roms::DumpReport, String> {
        Ok(crate::roms::analyze(&self.buffer_bytes()?))
    }

    pub fn rom_ident(&self) -> Result<crate::roms::RomIdentReport, String> {
        let bytes = self.buffer_bytes()?;
        let path = {
            let session = lock(&self.session);
            session.buffer_path.clone()
        };
        let mut report = crate::roms::mame_romident(&path)?;
        let notes = crate::roms::analyze(&bytes).lines;
        report.lines.splice(0..0, notes);
        Ok(report)
    }

    pub fn compare_buffer(&self, path: PathBuf) -> Result<crate::roms::DumpReport, String> {
        let dump = self.buffer_bytes()?;
        let other =
            fs::read(&path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file");
        let mut report = crate::roms::analyze(&dump);
        report.lines = crate::roms::compare(&dump, &other, name);
        Ok(report)
    }

    fn buffer_bytes(&self) -> Result<Vec<u8>, String> {
        let path = {
            let session = lock(&self.session);
            if session.buffer_len == 0 {
                return Err("Read a chip or open a file first.".into());
            }
            session.buffer_path.clone()
        };
        let data = fs::read(&path).map_err(|err| format!("cannot read buffer: {err}"))?;
        if data.is_empty() {
            return Err("The buffer is empty.".into());
        }
        Ok(data)
    }

    pub fn write_buffer(
        &self,
        app: &AppHandle,
        name: &str,
        page: u8,
        confirm: bool,
    ) -> Result<OpReport, String> {
        let mut guard = self.busy_guard()?;
        let handle = guard.handle.as_mut().expect("handle");
        let result = (|| {
            if !confirm {
                return Err(Fail::Soft("Write requires confirmation.".into()));
            }
            let check_id = self.prepare(handle);
            let device = lookup(&self.db, handle, name)?;
            let memory = page_size(&device, page).map_err(Fail::Soft)?;
            let image = self.buffer_image(memory).map_err(Fail::Soft)?;
            let len = image.len();
            log_line(
                app,
                format!(
                    "Writing buffer ({len} bytes) to {name} ({})...",
                    page_name(page)
                ),
            );
            let app = app.clone();
            let stats = transaction(handle, device, |handle| {
                let mut progress = make_progress(app.clone());
                write_chip_bytes(
                    handle,
                    image,
                    page,
                    SizeMismatch::Error,
                    false,
                    check_id,
                    Some(&mut progress),
                )
            })
            .map_err(classify)?;
            let message = format!("Wrote {} bytes, CRC32 {:08x}.", stats.bytes, stats.crc32);
            log_line(&app, &message);
            Ok(report(
                message,
                stats.bytes as u64,
                Some(stats.crc32),
                None,
                buffer_len(&self.session),
            ))
        })();
        guard.fatal = is_fatal(&result);
        result.map_err(fail_message)
    }

    pub fn verify_buffer(&self, app: &AppHandle, name: &str, page: u8) -> Result<OpReport, String> {
        let mut guard = self.busy_guard()?;
        let handle = guard.handle.as_mut().expect("handle");
        let result = (|| {
            let check_id = self.prepare(handle);
            let device = lookup(&self.db, handle, name)?;
            let memory = page_size(&device, page).map_err(Fail::Soft)?;
            let image = self.buffer_image(memory).map_err(Fail::Soft)?;
            let len = image.len() as u64;
            log_line(
                app,
                format!(
                    "Verifying buffer ({len} bytes) against {name} ({})...",
                    page_name(page)
                ),
            );
            let app = app.clone();
            transaction(handle, device, |handle| {
                let mut progress = make_progress(app.clone());
                verify_chip_bytes(handle, image, page, check_id, Some(&mut progress))
            })
            .map_err(classify)?;
            let message = format!("Verify passed ({len} bytes).");
            log_line(&app, &message);
            Ok(report(message, len, None, None, buffer_len(&self.session)))
        })();
        guard.fatal = is_fatal(&result);
        result.map_err(fail_message)
    }

    pub fn read_config(&self, app: &AppHandle, name: &str) -> Result<Vec<FuseReading>, String> {
        let mut guard = self.busy_guard()?;
        let handle = guard.handle.as_mut().expect("handle");
        let result = (|| {
            self.prepare(handle);
            let device = lookup(&self.db, handle, name)?;
            let fields = fuse_fields(&device);
            if fields.is_empty() {
                return Err(Fail::Soft("This device has no fuse configuration.".into()));
            }
            log_line(app, format!("Reading configuration for {name}..."));
            let values =
                transaction(handle, device, |handle| read_fuses(handle)).map_err(classify)?;
            let readings = fields
                .into_iter()
                .map(|field| {
                    let value = values
                        .iter()
                        .find(|item| item.name == field.name)
                        .map(|item| item.value)
                        .unwrap_or(0xffff);
                    FuseReading {
                        name: field.name,
                        value,
                        mask: field.mask,
                        group: field.group,
                    }
                })
                .collect();
            log_line(app, "Configuration read.");
            Ok(readings)
        })();
        guard.fatal = is_fatal(&result);
        result.map_err(fail_message)
    }

    pub fn write_config(
        &self,
        app: &AppHandle,
        name: &str,
        updates: &[FuseWrite],
        confirm: bool,
    ) -> Result<OpReport, String> {
        let mut guard = self.busy_guard()?;
        let handle = guard.handle.as_mut().expect("handle");
        let result = (|| {
            if !confirm {
                return Err(Fail::Soft(
                    "Writing configuration requires confirmation.".into(),
                ));
            }
            self.prepare(handle);
            let device = lookup(&self.db, handle, name)?;
            let fields = fuse_fields(&device);
            if fields.is_empty() {
                return Err(Fail::Soft("This device has no fuse configuration.".into()));
            }
            let fuses = fields
                .iter()
                .map(|field| {
                    let value = updates
                        .iter()
                        .find(|item| item.name == field.name)
                        .map(|item| item.value)
                        .ok_or_else(|| Fail::Soft(format!("Missing value for {}.", field.name)))?;
                    Ok(FuseValue {
                        name: field.name.clone(),
                        value,
                    })
                })
                .collect::<Result<Vec<_>, Fail>>()?;
            log_line(app, format!("Writing configuration for {name}..."));
            transaction(handle, device, |handle| write_fuses(handle, &fuses)).map_err(classify)?;
            let message = format!("Wrote {} configuration fields.", fuses.len());
            log_line(&app, &message);
            Ok(report(message, 0, None, None, buffer_len(&self.session)))
        })();
        guard.fatal = is_fatal(&result);
        result.map_err(fail_message)
    }

    pub fn logic_test(&self, app: &AppHandle, name: &str) -> Result<LogicReport, String> {
        let mut guard = self.busy_guard()?;
        let handle = guard.handle.as_mut().expect("handle");
        let result = (|| {
            self.prepare(handle);
            let device = lookup(&self.db, handle, name)?;
            if device.chip_type != 5 {
                return Err(Fail::Soft("Logic test only applies to logic ICs.".into()));
            }
            log_line(app, format!("Testing logic IC {name}..."));
            let report =
                transaction(handle, device, |handle| logic_ic_test(handle)).map_err(classify)?;
            let message = if report.pass {
                format!(
                    "Logic test passed ({} pins, {} vectors).",
                    report.pin_count, report.vector_count
                )
            } else {
                format!(
                    "Logic test failed: {} pin errors across {} vectors.",
                    report.errors, report.vector_count
                )
            };
            log_line(&app, &message);
            Ok(LogicReport {
                pass: report.pass,
                errors: report.errors,
                pin_count: report.pin_count,
                vector_count: report.vector_count,
                message,
            })
        })();
        guard.fatal = is_fatal(&result);
        result.map_err(fail_message)
    }

    pub fn pin_check(&self, app: &AppHandle, name: &str) -> Result<PinReport, String> {
        let mut guard = self.busy_guard()?;
        let handle = guard.handle.as_mut().expect("handle");
        let infoic = self.db.infoic.clone();
        let result = (|| {
            self.prepare(handle);
            let device = lookup(&self.db, handle, name)?;
            if device.pin_map & 0xff == 0 {
                return Err(Fail::Soft(
                    "Pin contact check is not available for this device.".into(),
                ));
            }
            log_line(app, format!("Checking pin contact for {name}..."));
            let report = transaction(handle, device, |handle| pin_contact_check(handle, &infoic))
                .map_err(classify)?;
            let message = if report.bad_pins.is_empty() {
                "All tested pins made contact.".into()
            } else {
                let pins = report
                    .bad_pins
                    .iter()
                    .map(|pin| pin.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut message = format!("Poor contact on pin {pins}.");
                if report.bad_pins.len() >= 8 {
                    message.push_str(
                        " Seat a DIP at the bottom of the ZIF, against the lever, and close it firmly. Open address or data pins make every read return a different CRC.",
                    );
                }
                message
            };
            log_line(&app, &message);
            Ok(PinReport {
                message,
                bad_pins: report.bad_pins,
            })
        })();
        guard.fatal = is_fatal(&result);
        result.map_err(fail_message)
    }

    pub fn hardware_test(&self, app: &AppHandle) -> Result<OpReport, String> {
        let mut guard = self.busy_guard()?;
        let handle = guard.handle.as_mut().expect("handle");
        let result = (|| {
            log_line(app, "Running programmer hardware test...");
            minipro_core::operations::hardware_check(handle).map_err(classify)?;
            let message = "Hardware test passed.".to_string();
            log_line(&app, &message);
            Ok(report(message, 0, None, None, buffer_len(&self.session)))
        })();
        guard.fatal = is_fatal(&result);
        result.map_err(fail_message)
    }

    pub fn spi_detect(&self, app: &AppHandle, pins: u8) -> Result<SpiReport, String> {
        let id_type = match pins {
            8 => 0,
            16 => 1,
            _ => return Err("SPI detect supports 8-pin or 16-pin packages.".into()),
        };
        let mut guard = self.busy_guard()?;
        let handle = guard.handle.as_mut().expect("handle");
        let result = (|| {
            log_line(app, format!("Detecting {pins}-pin SPI device..."));
            let found = spi_autodetect_and_lookup(handle, &self.db, id_type).map_err(classify)?;
            let matches = found
                .matches
                .into_iter()
                .take(20)
                .map(|item| SearchHit {
                    name: item.name,
                    manufacturer: item.manufacturer,
                })
                .collect::<Vec<_>>();
            let message = if matches.is_empty() {
                format!("JEDEC ID {:#010x}, no database match.", found.jedec_id)
            } else {
                format!(
                    "JEDEC ID {:#010x}, {} match{}.",
                    found.jedec_id,
                    matches.len(),
                    if matches.len() == 1 { "" } else { "es" }
                )
            };
            log_line(&app, &message);
            Ok(SpiReport {
                jedec_id: format!("{:#010x}", found.jedec_id),
                matches,
            })
        })();
        guard.fatal = is_fatal(&result);
        result.map_err(fail_message)
    }

    pub fn update_firmware(
        &self,
        app: &AppHandle,
        path: &Path,
        confirm: bool,
    ) -> Result<OpReport, String> {
        let mut guard = self.busy_guard()?;
        let handle = guard.handle.as_mut().expect("handle");
        let mut started = false;
        let result = (|| {
            if !confirm {
                return Err(Fail::Soft("Firmware update requires confirmation.".into()));
            }
            let data =
                fs::read(path).map_err(|err| Fail::Soft(format!("cannot read firmware: {err}")))?;
            if data.is_empty() {
                return Err(Fail::Soft("Firmware file is empty.".into()));
            }
            started = true;
            log_line(
                app,
                "Updating firmware. Leave the programmer connected until this finishes.",
            );
            let app = app.clone();
            let mut sink = Vec::new();
            let mut progress = make_progress(app.clone());
            firmware_update(handle, &data, &mut sink, Some(&mut progress)).map_err(classify)?;
            let note = String::from_utf8_lossy(&sink);
            let message = if note.trim().is_empty() {
                "Firmware update finished. Rescan the programmer.".to_string()
            } else {
                format!("Firmware update finished. {}", note.trim())
            };
            log_line(&app, &message);
            Ok(report(message, data.len() as u64, None, None, 0))
        })();
        guard.fatal = started || is_fatal(&result);
        result.map_err(fail_message)
    }

    fn prepare(&self, handle: &mut MiniproHandle) -> bool {
        let session = lock(&self.session);
        handle.icsp = session.icsp;
        !session.skip_id
    }

    fn buffer_image(&self, memory: usize) -> Result<Vec<u8>, String> {
        let (path, len) = {
            let session = lock(&self.session);
            (session.buffer_path.clone(), session.buffer_len)
        };
        if len as usize != memory {
            return Err(format!(
                "Buffer is {len} bytes, but this memory is {memory} bytes."
            ));
        }
        let data = fs::read(&path).map_err(|err| format!("cannot read buffer: {err}"))?;
        if data.len() != memory {
            return Err(format!(
                "Buffer is {} bytes, but this memory is {memory} bytes.",
                data.len()
            ));
        }
        Ok(data)
    }

    fn busy_guard(&self) -> Result<BusyGuard<'_>, String> {
        let mut session = lock(&self.session);
        if session.busy {
            return Err("Another operation is already running.".into());
        }
        let handle = session
            .handle
            .take()
            .ok_or_else(|| "No programmer connected. Rescan and try again.".to_string())?;
        session.busy = true;
        Ok(BusyGuard {
            session: &self.session,
            handle: Some(handle),
            fatal: false,
        })
    }

    fn ensure_index(&self, model: ProgrammerModel) -> Result<(), String> {
        if lock(&self.index).contains_key(&model) {
            return Ok(());
        }
        let items = list_devices_for_model(&self.db, None, model).map_err(|e| e.to_string())?;
        lock(&self.index).insert(model, items);
        Ok(())
    }

    fn store_buffer(&self, bytes: &[u8]) -> Result<(), String> {
        let mut session = lock(&self.session);
        if session.busy {
            return Err("Another operation is already running.".into());
        }
        fs::write(&session.buffer_path, bytes).map_err(|e| format!("cannot write buffer: {e}"))?;
        session.buffer_len = bytes.len() as u64;
        Ok(())
    }
}

fn lookup(db: &DatabasePaths, handle: &MiniproHandle, name: &str) -> Result<Device, Fail> {
    find_device(db, name, handle.info.model).map_err(|e| Fail::Soft(e.to_string()))
}

fn transaction<T>(
    handle: &mut MiniproHandle,
    device: Device,
    f: impl FnOnce(&mut MiniproHandle) -> Result<T, MiniproError>,
) -> Result<T, MiniproError> {
    if let Err(err) = handle.begin_transaction(Arc::new(device)) {
        let _ = handle.end_transaction();
        return Err(err);
    }
    let result = f(handle);
    if let Err(err) = handle.end_transaction() {
        return result.and(Err(err));
    }
    result
}

fn image_for(path: &Path, device: &Device, page: u8) -> Result<Vec<u8>, String> {
    let memory = page_size(device, page)?;
    let file_bytes = fs::metadata(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        .len();
    if file_bytes == 0 {
        return Err("File is empty.".into());
    }
    let format = format_for(path);
    if format == "bin" && file_bytes != memory as u64 {
        return Err(format!(
            "File is {file_bytes} bytes, but this memory is {memory} bytes."
        ));
    }
    let buf =
        read_file(path, format, memory, device.blank_value as u8).map_err(|e| e.to_string())?;
    if buf.len() != memory {
        return Err(format!(
            "File decoded to {} bytes, but this memory is {memory} bytes.",
            buf.len()
        ));
    }
    Ok(buf)
}

fn page_size(device: &Device, page: u8) -> Result<usize, String> {
    match page {
        0 => Ok(device.code_memory_size as usize),
        1 if device.data_memory_size > 0 => Ok(device.data_memory_size as usize),
        1 => Err("This device has no data memory.".into()),
        _ => Err("Unknown memory page.".into()),
    }
}

fn page_name(page: u8) -> &'static str {
    match page {
        0 => "code",
        1 => "data",
        _ => "memory",
    }
}

fn device_info(name: &str, device: &Device) -> DeviceInfo {
    let package = name
        .split_once('@')
        .map(|(_, package)| package.to_string())
        .filter(|package| !package.is_empty())
        .unwrap_or_else(|| {
            if device.package_details.pin_count > 0 {
                format!("{}-pin", device.package_details.pin_count)
            } else {
                "unknown".into()
            }
        });
    let chip_id = if device.chip_id == 0 {
        "none".into()
    } else {
        format!("{:#010x}", device.chip_id)
    };
    let blank_value = if device.blank_value <= 0xff {
        format!("{:#04x}", device.blank_value)
    } else {
        format!("{:#06x}", device.blank_value)
    };
    let fuses = fuse_fields(device);
    DeviceInfo {
        name: name.to_string(),
        manufacturer: device.manufacturer.clone(),
        package,
        code_bytes: device.code_memory_size,
        data_bytes: device.data_memory_size,
        chip_id,
        has_chip_id: device.flags.has_chip_id,
        blank_value,
        can_erase: device.flags.can_erase,
        kind: kind_name(device.chip_type).to_string(),
        fuses,
    }
}

fn kind_name(chip_type: u32) -> &'static str {
    match chip_type {
        1 => "memory",
        2 => "mcu",
        3 => "pld",
        4 => "sram",
        5 => "logic",
        6 => "nand",
        7 => "emmc",
        8 => "vga",
        _ => "other",
    }
}

fn fuse_fields(device: &Device) -> Vec<FuseFieldInfo> {
    let Some(ChipConfig::Mcu(config)) = device.config.as_ref() else {
        return Vec::new();
    };
    let mut fields = Vec::new();
    for field in &config.fuses {
        fields.push(FuseFieldInfo {
            name: field.name.clone(),
            mask: field.mask,
            default_value: field.default,
            group: "fuse".into(),
        });
    }
    for field in &config.locks {
        fields.push(FuseFieldInfo {
            name: field.name.clone(),
            mask: field.mask,
            default_value: field.default,
            group: "lock".into(),
        });
    }
    fields
}

fn crc_hex(data: &[u8]) -> String {
    format!(
        "{:08x}",
        crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC).checksum(data)
    )
}

fn report(
    message: String,
    bytes: u64,
    crc32: Option<u32>,
    chip_id: Option<String>,
    buffer_length: u64,
) -> OpReport {
    OpReport {
        message,
        bytes,
        crc32: crc32.map(|value| format!("{value:08x}")),
        chip_id,
        buffer_length,
    }
}

fn classify(err: MiniproError) -> Fail {
    let fatal = err.is_usb_communication_error();
    let message = err.to_string();
    if fatal {
        Fail::Fatal(message)
    } else {
        Fail::Soft(message)
    }
}

fn is_fatal<T>(result: &Result<T, Fail>) -> bool {
    matches!(result, Err(Fail::Fatal(_)))
}

fn fail_message(err: Fail) -> String {
    match err {
        Fail::Soft(message) | Fail::Fatal(message) => message,
    }
}

fn buffer_len(session: &Mutex<Session>) -> u64 {
    lock(session).buffer_len
}

fn status_from(session: &Session, busy: bool, warning: Option<String>) -> ProgrammerStatusDto {
    match &session.last_info {
        Some(info) => ProgrammerStatusDto {
            connected: true,
            busy,
            model: Some(info.model.clone()),
            firmware: Some(info.firmware.clone()),
            serial_number: Some(info.serial_number.clone()),
            message: format!("Connected to {}, firmware {}.", info.model, info.firmware),
            warning,
        },
        None => ProgrammerStatusDto {
            connected: false,
            busy,
            model: None,
            firmware: None,
            serial_number: None,
            message: "No programmer found.".into(),
            warning: None,
        },
    }
}

fn disconnected(err: MiniproError) -> ProgrammerStatusDto {
    let message = match err {
        MiniproError::NoProgrammerFound => "No programmer found.".into(),
        other => other.to_string(),
    };
    ProgrammerStatusDto {
        connected: false,
        busy: false,
        model: None,
        firmware: None,
        serial_number: None,
        message,
        warning: None,
    }
}

fn algorithm_warning(model: ProgrammerModel, db: &DatabasePaths) -> Option<String> {
    if matches!(model, ProgrammerModel::T56 | ProgrammerModel::T76) && db.algorithms.is_none() {
        Some(
            "No algorithm.xml found. Some T56 and T76 chips need an FPGA bitstream installed for the C minipro tool."
                .into(),
        )
    } else {
        None
    }
}

fn parse_model(model: &str) -> Result<ProgrammerModel, String> {
    model.parse()
}

fn format_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("hex" | "ihx" | "ihex") => "ihex",
        Some("srec" | "s19" | "s28" | "s37" | "mot") => "srec",
        Some("jed") => "jedec",
        _ => "bin",
    }
}

fn log_line(app: &AppHandle, message: impl AsRef<str>) {
    let message = message.as_ref().to_string();
    eprintln!("miniproui: {message}");
    let _ = app.emit("log", LogEvent { message });
}

fn make_progress(app: AppHandle) -> impl FnMut(usize, usize) {
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let mut last_pct = u16::MAX;
    move |done, total| {
        let pct = if total == 0 {
            100
        } else {
            ((done.saturating_mul(100)) / total) as u16
        };
        let finished = total > 0 && done >= total;
        if pct != last_pct && (finished || last_emit.elapsed() >= Duration::from_millis(80)) {
            last_pct = pct;
            last_emit = Instant::now();
            let _ = app.emit(
                "progress",
                ProgressEvent {
                    done: done as u64,
                    total: total as u64,
                },
            );
        }
    }
}

fn database_paths() -> Result<DatabasePaths, String> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let data = manifest.join("../third_party/minipro-rs/data");
    let infoic = data.join("infoic.xml");
    let logicic = data.join("logicic.xml");
    if infoic.is_file() && logicic.is_file() {
        return Ok(DatabasePaths {
            infoic,
            logicic,
            algorithms: algorithm_path(&data),
        });
    }
    DatabasePaths::resolve(None, None, None).map_err(|e| e.to_string())
}

fn algorithm_path(data: &Path) -> Option<PathBuf> {
    let mut candidates = vec![
        data.join("algorithm.xml"),
        PathBuf::from("/opt/homebrew/share/minipro/algorithm.xml"),
        PathBuf::from("/usr/local/share/minipro/algorithm.xml"),
    ];
    if let Ok(entries) = fs::read_dir("/opt/homebrew/Cellar/minipro") {
        for entry in entries.flatten() {
            candidates.push(entry.path().join("share/minipro/algorithm.xml"));
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_search_returns_nothing() {
        let state = AppState::new().expect("state");
        let response = state.search("a", "TL866II+").unwrap();
        assert!(response.hits.is_empty());
        assert!(!response.truncated);
    }

    #[test]
    fn search_and_file_size_check() {
        let state = AppState::new().expect("state");
        let response = state.search("atmega48@dip28", "TL866II+").unwrap();
        assert!(
            response
                .hits
                .iter()
                .any(|hit| hit.name.eq_ignore_ascii_case("ATMEGA48@DIP28")),
            "expected ATMEGA48@DIP28 in {:?}",
            response
                .hits
                .iter()
                .map(|hit| &hit.name)
                .collect::<Vec<_>>()
        );

        let info = state
            .device_info("ATMEGA48@DIP28", "TL866II+")
            .expect("device");
        assert!(info.code_bytes > 16);
        assert!(!info.package.is_empty());

        let mismatch = std::env::temp_dir().join("miniproui-test-mismatch.bin");
        fs::write(&mismatch, [0u8; 16]).unwrap();
        let report = state
            .load_file(&mismatch, Some("ATMEGA48@DIP28"), 0, "TL866II+")
            .unwrap();
        assert!(!report.matches);
        assert_eq!(report.buffer_length, 16);
        assert!(report.message.contains("this memory is"));

        let exact = std::env::temp_dir().join("miniproui-test-exact.bin");
        fs::write(&exact, vec![0xA5u8; info.code_bytes as usize]).unwrap();
        let report = state
            .load_file(&exact, Some("ATMEGA48@DIP28"), 0, "TL866II+")
            .unwrap();
        assert!(report.matches, "{}", report.message);
        assert_eq!(report.buffer_length, info.code_bytes as u64);

        let window = state.hex_window(0, 16).unwrap();
        assert_eq!(window.bytes, vec![0xA5u8; 16]);
        assert!(!info.fuses.is_empty(), "ATMEGA48 should expose fuse fields");
        let crc = state.patch_buffer(0, &[0x11]).unwrap();
        assert!(!crc.is_empty());
        let window = state.hex_window(0, 1).unwrap();
        assert_eq!(window.bytes, vec![0x11]);
        let _ = fs::remove_file(&mismatch);
        let _ = fs::remove_file(&exact);
    }

    #[test]
    fn programmer_status_does_not_panic() {
        let state = AppState::new().expect("state");
        let status = state.refresh_programmer(true);
        assert!(!status.message.is_empty());
        if !status.connected {
            assert!(status.model.is_none());
        }
    }
}
