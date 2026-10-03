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
use minipro_core::device::{Device, ProgrammerModel};
use minipro_core::error::MiniproError;
use minipro_core::handle::MiniproHandle;
use minipro_core::operations::{
    blank_check, erase_chip, read_chip, read_file, verify_chip_bytes, write_chip_bytes, write_file,
    SizeMismatch,
};
use serde::Serialize;
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
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileReport {
    pub matches: bool,
    pub message: String,
    pub buffer_length: u64,
    pub memory_bytes: u64,
    pub file_bytes: u64,
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
                read_chip(handle, &path, page, "bin", true, Some(&mut progress))
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
                    true,
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
                verify_chip_bytes(handle, image, page, true, Some(&mut progress))
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
            let device = lookup(&self.db, handle, name)?;
            if !device.flags.can_erase {
                return Err(Fail::Soft(
                    "This device cannot be electrically erased.".into(),
                ));
            }
            log_line(app, format!("Erasing {name}..."));
            transaction(handle, device, |handle| erase_chip(handle, true)).map_err(classify)?;
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
    }
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
