//! Programmer session for the browser build.
//!
//! The buffer lives in memory. Chip-database XML is mounted into the virtual
//! filesystem before any lookup.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use minipro_core::database::{find_device, list_devices_for_model, DatabasePaths, DeviceListItem};
use minipro_core::device::{ChipConfig, Device, ProgrammerModel, ICSP_ENABLE, ICSP_VCC};
use minipro_core::error::MiniproError;
use minipro_core::format::{ihex, jedec, srec};
use minipro_core::handle::MiniproHandle;
use minipro_core::operations::{
    blank_check, erase_chip, logic_ic_test, pin_contact_check, read_chip, read_fuses,
    spi_autodetect_and_lookup, verify_chip_bytes, write_chip_bytes, write_fuses, FuseValue,
    SizeMismatch,
};
use minipro_core::usb::{model_from_ids, UsbDevice};
use minipro_core::vfs;
use serde::Serialize;
use sha1::{Digest, Sha1};

use crate::host::{self, JsHost};

const SEARCH_LIMIT: usize = 200;
const HEX_WINDOW_MAX: u32 = 8192;
const READ_PATH: &str = "/minipro/read-buffer.bin";

pub struct State {
    db: Option<DatabasePaths>,
    handle: Option<MiniproHandle>,
    last: Option<LastInfo>,
    buffer: Vec<u8>,
    skip_id: bool,
    icsp: u8,
    index: HashMap<ProgrammerModel, Vec<DeviceListItem>>,
}

struct LastInfo {
    model: String,
    firmware: String,
    serial_number: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgrammerStatus {
    connected: bool,
    busy: bool,
    model: Option<String>,
    firmware: Option<String>,
    serial_number: Option<String>,
    message: String,
    warning: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchHit {
    name: String,
    manufacturer: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchResponse {
    hits: Vec<SearchHit>,
    truncated: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceInfo {
    name: String,
    manufacturer: String,
    package: String,
    code_bytes: u32,
    data_bytes: u32,
    chip_id: String,
    has_chip_id: bool,
    blank_value: String,
    can_erase: bool,
    kind: String,
    fuses: Vec<FuseFieldInfo>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FuseFieldInfo {
    name: String,
    mask: u16,
    default_value: u16,
    group: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FuseReading {
    name: String,
    value: u16,
    mask: u16,
    group: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LogicReport {
    pass: bool,
    errors: u32,
    pin_count: u16,
    vector_count: u16,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PinReport {
    message: String,
    bad_pins: Vec<u16>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SpiReport {
    jedec_id: String,
    matches: Vec<SearchHit>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileReport {
    matches: bool,
    message: String,
    buffer_length: u64,
    memory_bytes: u64,
    file_bytes: u64,
    crc32: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpReport {
    message: String,
    bytes: u64,
    crc32: Option<String>,
    chip_id: Option<String>,
    buffer_length: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HexWindow {
    offset: u64,
    length: u64,
    bytes: Vec<u8>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DumpReport {
    crc32: String,
    sha1: String,
    length: u64,
    lines: Vec<String>,
}

enum Fail {
    Soft(String),
    Fatal(String),
}

impl State {
    pub fn new() -> Self {
        Self {
            db: None,
            handle: None,
            last: None,
            buffer: Vec::new(),
            skip_id: false,
            icsp: 0,
            index: HashMap::new(),
        }
    }

    pub fn mount(&mut self, infoic: &str, logicic: &str, algorithms: Option<&str>) {
        let infoic_path = PathBuf::from("/minipro/infoic.xml");
        let logicic_path = PathBuf::from("/minipro/logicic.xml");
        vfs::mount(&infoic_path, infoic.as_bytes());
        vfs::mount(&logicic_path, logicic.as_bytes());
        let algorithms = algorithms.map(|text| {
            let path = PathBuf::from("/minipro/algorithm.xml");
            vfs::mount(&path, text.as_bytes());
            path
        });
        self.db = Some(DatabasePaths {
            infoic: infoic_path,
            logicic: logicic_path,
            algorithms,
        });
        self.index.clear();
    }

    pub fn programmer_status(&mut self, force: bool, vid: u16, pid: u16) -> Result<String, String> {
        if !force && self.handle.is_some() {
            return Ok(json(&self.status(false)));
        }
        if !force {
            return Ok(json(&disconnected("Click Rescan to connect a programmer.")));
        }
        let initial = model_from_ids(vid, pid).ok_or_else(|| {
            format!("USB device {vid:04x}:{pid:04x} is not a supported programmer.")
        })?;
        let db = self.db.clone().ok_or("Chip database is still loading.")?;
        let usb = UsbDevice::from_host(pid, Arc::new(JsHost));
        match MiniproHandle::from_usb(usb, initial, false) {
            Ok(mut handle) => {
                handle.db_paths = Some(db.clone());
                handle.icsp = self.icsp;
                self.last = Some(LastInfo {
                    model: handle.info.model.to_string(),
                    firmware: handle.info.firmware_str.clone(),
                    serial_number: handle.info.serial_number.clone(),
                });
                self.handle = Some(handle);
                let status = self.status(false);
                host::log(&status.message);
                if let Some(warning) = &status.warning {
                    host::log(warning);
                }
                Ok(json(&status))
            }
            Err(err) => {
                self.handle = None;
                self.last = None;
                let message = err.to_string();
                host::log(&message);
                Ok(json(&disconnected(&message)))
            }
        }
    }

    pub fn search(&mut self, query: &str, model: &str) -> Result<String, String> {
        let query = query.trim();
        if query.len() < 2 {
            return Ok(json(&SearchResponse {
                hits: Vec::new(),
                truncated: false,
            }));
        }
        let model = parse_model(model)?;
        self.ensure_index(model)?;
        let items = self
            .index
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
        Ok(json(&SearchResponse { hits, truncated }))
    }

    pub fn device_info(&self, name: &str, model: &str) -> Result<String, String> {
        let model = parse_model(model)?;
        let db = self.db.as_ref().ok_or("Chip database is still loading.")?;
        let device = find_device(db, name, model).map_err(|err| err.to_string())?;
        Ok(json(&device_info(name, &device)))
    }

    pub fn load_bytes(
        &mut self,
        name: &str,
        filename: &str,
        page: u8,
        model: &str,
        bytes: &[u8],
    ) -> Result<String, String> {
        if bytes.is_empty() {
            return Err("File is empty.".into());
        }
        let file_bytes = bytes.len() as u64;
        let format = format_for(Path::new(filename));
        if name.is_empty() {
            if format != "bin" {
                return Err("Select a device before loading an Intel HEX or S-record file.".into());
            }
            self.buffer = bytes.to_vec();
            let len = self.buffer.len() as u64;
            return Ok(json(&FileReport {
                matches: true,
                message: format!("Loaded {len} bytes."),
                buffer_length: len,
                memory_bytes: 0,
                file_bytes,
                crc32: crc_hex(&self.buffer),
            }));
        }
        let model = parse_model(model)?;
        let db = self.db.as_ref().ok_or("Chip database is still loading.")?;
        let device = find_device(db, name, model).map_err(|err| err.to_string())?;
        let memory = page_size(&device, page)?;
        if format == "bin" {
            let matches = bytes.len() == memory;
            self.buffer = bytes.to_vec();
            let message = if matches {
                format!("Loaded {} bytes.", bytes.len())
            } else {
                format!(
                    "File is {} bytes, but this memory is {memory} bytes.",
                    bytes.len()
                )
            };
            return Ok(json(&FileReport {
                matches,
                message,
                buffer_length: self.buffer.len() as u64,
                memory_bytes: memory as u64,
                file_bytes,
                crc32: crc_hex(&self.buffer),
            }));
        }
        let decoded = decode(format, bytes, memory, device.blank_value as u8)?;
        let matches = decoded.len() == memory;
        let message = format!("Decoded {format} into {} bytes.", decoded.len());
        self.buffer = decoded;
        Ok(json(&FileReport {
            matches,
            message,
            buffer_length: self.buffer.len() as u64,
            memory_bytes: memory as u64,
            file_bytes,
            crc32: crc_hex(&self.buffer),
        }))
    }

    pub fn hex_window(&self, offset: u64, length: u32) -> Result<String, String> {
        let length = length.min(HEX_WINDOW_MAX) as u64;
        let total = self.buffer.len() as u64;
        if length == 0 || total == 0 || offset >= total {
            return Ok(json(&HexWindow {
                offset,
                length: total,
                bytes: Vec::new(),
            }));
        }
        let end = (offset + length).min(total) as usize;
        Ok(json(&HexWindow {
            offset,
            length: total,
            bytes: self.buffer[offset as usize..end].to_vec(),
        }))
    }

    pub fn encode(&self, filename: &str) -> Result<Vec<u8>, String> {
        if self.buffer.is_empty() {
            return Err("Nothing to save.".into());
        }
        encode(filename, &self.buffer)
    }

    pub fn patch(&mut self, offset: u64, bytes: &[u8]) -> Result<String, String> {
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| "Patch is outside the buffer.".to_string())?;
        if end > self.buffer.len() as u64 {
            return Err("Patch is outside the buffer.".into());
        }
        let start = offset as usize;
        self.buffer[start..start + bytes.len()].copy_from_slice(bytes);
        serde_json::to_string(&crc_hex(&self.buffer)).map_err(|err| err.to_string())
    }

    pub fn buffer_report(&self) -> Result<String, String> {
        if self.buffer.is_empty() {
            return Err("Read a chip or open a file first.".into());
        }
        Ok(json(&analyze(&self.buffer)))
    }

    pub fn compare(&self, name: &str, other: &[u8]) -> Result<String, String> {
        if self.buffer.is_empty() {
            return Err("Read a chip or open a file first.".into());
        }
        let mut report = analyze(&self.buffer);
        report.lines = compare(&self.buffer, other, name);
        Ok(json(&report))
    }

    pub fn read_memory(&mut self, name: &str, page: u8) -> Result<String, String> {
        self.operate(|state| {
            let check_id = state.prepare();
            let device = state.lookup(name)?;
            let memory = page_size(&device, page).map_err(Fail::Soft)?;
            host::log(format!(
                "Reading {memory} bytes of {name} ({})...",
                page_name(page)
            ));
            let path = PathBuf::from(READ_PATH);
            let stats = {
                let handle = state.handle.as_mut().expect("handle");
                transaction(handle, device, |handle| {
                    let mut progress = |done, total| host::progress(done, total);
                    read_chip(handle, &path, page, "bin", check_id, Some(&mut progress))
                })
                .map_err(classify)?
            };
            state.buffer = vfs::read(&path).map_err(|err| Fail::Soft(err.to_string()))?;
            host::progress(stats.bytes, stats.bytes);
            let message = format!("Read {} bytes, CRC32 {:08x}.", stats.bytes, stats.crc32);
            host::log(&message);
            Ok(report(
                message,
                stats.bytes as u64,
                Some(stats.crc32),
                None,
                state.buffer.len() as u64,
            ))
        })
    }

    pub fn write_buffer(&mut self, name: &str, page: u8, confirm: bool) -> Result<String, String> {
        self.operate(|state| {
            if !confirm {
                return Err(Fail::Soft("Write requires confirmation.".into()));
            }
            let check_id = state.prepare();
            let device = state.lookup(name)?;
            let memory = page_size(&device, page).map_err(Fail::Soft)?;
            let image = state.buffer_image(memory).map_err(Fail::Soft)?;
            let len = image.len();
            host::log(format!(
                "Writing buffer ({len} bytes) to {name} ({})...",
                page_name(page)
            ));
            let stats = {
                let handle = state.handle.as_mut().expect("handle");
                transaction(handle, device, |handle| {
                    let mut progress = |done, total| host::progress(done, total);
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
                .map_err(classify)?
            };
            let message = format!("Wrote {} bytes, CRC32 {:08x}.", stats.bytes, stats.crc32);
            host::log(&message);
            Ok(report(
                message,
                stats.bytes as u64,
                Some(stats.crc32),
                None,
                state.buffer.len() as u64,
            ))
        })
    }

    pub fn verify_buffer(&mut self, name: &str, page: u8) -> Result<String, String> {
        self.operate(|state| {
            let check_id = state.prepare();
            let device = state.lookup(name)?;
            let memory = page_size(&device, page).map_err(Fail::Soft)?;
            let image = state.buffer_image(memory).map_err(Fail::Soft)?;
            let len = image.len() as u64;
            host::log(format!(
                "Verifying buffer ({len} bytes) against {name} ({})...",
                page_name(page)
            ));
            {
                let handle = state.handle.as_mut().expect("handle");
                transaction(handle, device, |handle| {
                    let mut progress = |done, total| host::progress(done, total);
                    verify_chip_bytes(handle, image, page, check_id, Some(&mut progress))
                })
                .map_err(classify)?;
            }
            let message = format!("Verify passed ({len} bytes).");
            host::log(&message);
            Ok(report(
                message,
                len,
                None,
                None,
                state.buffer.len() as u64,
            ))
        })
    }

    pub fn erase(&mut self, name: &str, confirm: bool) -> Result<String, String> {
        self.operate(|state| {
            if !confirm {
                return Err(Fail::Soft("Erase requires confirmation.".into()));
            }
            let check_id = state.prepare();
            let device = state.lookup(name)?;
            if !device.flags.can_erase {
                return Err(Fail::Soft(
                    "This device cannot be electrically erased.".into(),
                ));
            }
            host::log(format!("Erasing {name}..."));
            {
                let handle = state.handle.as_mut().expect("handle");
                transaction(handle, device, |handle| erase_chip(handle, check_id))
                    .map_err(classify)?;
            }
            let message = format!("Erased {name}.");
            host::log(&message);
            Ok(report(message, 0, None, None, state.buffer.len() as u64))
        })
    }

    pub fn blank_check(&mut self, name: &str) -> Result<String, String> {
        self.operate(|state| {
            state.prepare();
            let device = state.lookup(name)?;
            host::log(format!("Blank-checking {name}..."));
            {
                let handle = state.handle.as_mut().expect("handle");
                transaction(handle, device, |handle| blank_check(handle)).map_err(classify)?;
            }
            let message = format!("{name} is blank.");
            host::log(&message);
            Ok(report(message, 0, None, None, state.buffer.len() as u64))
        })
    }

    pub fn read_id(&mut self, name: &str) -> Result<String, String> {
        self.operate(|state| {
            state.prepare();
            let device = state.lookup(name)?;
            if !device.flags.has_chip_id {
                return Err(Fail::Soft("This device does not support chip ID.".into()));
            }
            let expected = device.chip_id;
            host::log(format!("Reading chip ID for {name}..."));
            let actual = {
                let handle = state.handle.as_mut().expect("handle");
                transaction(handle, device, |handle| {
                    let current = handle.device()?.clone();
                    let (_kind, id) = handle.protocol.get_chip_id(&handle.usb, &current)?;
                    Ok(id)
                })
                .map_err(classify)?
            };
            let expected_text = if expected == 0 {
                "database has no expected ID".to_string()
            } else {
                format!("database {expected:#010x}")
            };
            let message = format!("Chip ID {actual:#010x} ({expected_text}).");
            host::log(&message);
            Ok(report(
                message,
                0,
                None,
                Some(format!("{actual:#010x}")),
                state.buffer.len() as u64,
            ))
        })
    }

    pub fn set_options(&mut self, skip_id: bool, icsp: &str) -> Result<String, String> {
        let icsp = match icsp {
            "off" | "zif" => 0,
            "no-vcc" => ICSP_ENABLE,
            "vcc" => ICSP_ENABLE | ICSP_VCC,
            other => return Err(format!("Unknown socket mode '{other}'.")),
        };
        self.skip_id = skip_id;
        self.icsp = icsp;
        if let Some(handle) = self.handle.as_mut() {
            handle.icsp = icsp;
        }
        Ok("null".into())
    }

    pub fn read_config(&mut self, name: &str) -> Result<String, String> {
        self.operate(|state| {
            state.prepare();
            let device = state.lookup(name)?;
            let fields = fuse_fields(&device);
            if fields.is_empty() {
                return Err(Fail::Soft("This device has no fuse configuration.".into()));
            }
            host::log(format!("Reading configuration for {name}..."));
            let values = {
                let handle = state.handle.as_mut().expect("handle");
                transaction(handle, device, |handle| read_fuses(handle)).map_err(classify)?
            };
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
                .collect::<Vec<_>>();
            host::log("Configuration read.");
            Ok(readings)
        })
    }

    pub fn write_config(
        &mut self,
        name: &str,
        updates: &[(String, u16)],
        confirm: bool,
    ) -> Result<String, String> {
        self.operate(|state| {
            if !confirm {
                return Err(Fail::Soft(
                    "Writing configuration requires confirmation.".into(),
                ));
            }
            state.prepare();
            let device = state.lookup(name)?;
            let fields = fuse_fields(&device);
            if fields.is_empty() {
                return Err(Fail::Soft("This device has no fuse configuration.".into()));
            }
            let fuses = fields
                .iter()
                .map(|field| {
                    let value = updates
                        .iter()
                        .find(|(item, _)| item == &field.name)
                        .map(|(_, value)| *value)
                        .ok_or_else(|| Fail::Soft(format!("Missing value for {}.", field.name)))?;
                    Ok(FuseValue {
                        name: field.name.clone(),
                        value,
                    })
                })
                .collect::<Result<Vec<_>, Fail>>()?;
            host::log(format!("Writing configuration for {name}..."));
            {
                let handle = state.handle.as_mut().expect("handle");
                transaction(handle, device, |handle| write_fuses(handle, &fuses))
                    .map_err(classify)?;
            }
            let message = format!("Wrote {} configuration fields.", fuses.len());
            host::log(&message);
            Ok(report(message, 0, None, None, state.buffer.len() as u64))
        })
    }

    pub fn logic_test(&mut self, name: &str) -> Result<String, String> {
        self.operate(|state| {
            state.prepare();
            let device = state.lookup(name)?;
            if device.chip_type != 5 {
                return Err(Fail::Soft("Logic test only applies to logic ICs.".into()));
            }
            host::log(format!("Testing logic IC {name}..."));
            let report = {
                let handle = state.handle.as_mut().expect("handle");
                transaction(handle, device, |handle| logic_ic_test(handle)).map_err(classify)?
            };
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
            host::log(&message);
            Ok(LogicReport {
                pass: report.pass,
                errors: report.errors,
                pin_count: report.pin_count,
                vector_count: report.vector_count,
                message,
            })
        })
    }

    pub fn pin_check(&mut self, name: &str) -> Result<String, String> {
        let infoic = self
            .db
            .as_ref()
            .ok_or("Chip database is still loading.")?
            .infoic
            .clone();
        self.operate(|state| {
            state.prepare();
            let device = state.lookup(name)?;
            if device.pin_map & 0xff == 0 {
                return Err(Fail::Soft(
                    "Pin contact check is not available for this device.".into(),
                ));
            }
            host::log(format!("Checking pin contact for {name}..."));
            let report = {
                let handle = state.handle.as_mut().expect("handle");
                transaction(handle, device, |handle| pin_contact_check(handle, &infoic))
                    .map_err(classify)?
            };
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
            host::log(&message);
            Ok(PinReport {
                message,
                bad_pins: report.bad_pins,
            })
        })
    }

    pub fn hardware_test(&mut self) -> Result<String, String> {
        self.operate(|state| {
            host::log("Running programmer hardware test...");
            let handle = state.handle.as_mut().expect("handle");
            minipro_core::operations::hardware_check(handle).map_err(classify)?;
            let message = "Hardware test passed.".to_string();
            host::log(&message);
            Ok(report(message, 0, None, None, state.buffer.len() as u64))
        })
    }

    pub fn spi_detect(&mut self, pins: u8) -> Result<String, String> {
        let id_type = match pins {
            8 => 0,
            16 => 1,
            _ => return Err("SPI detect supports 8-pin or 16-pin packages.".into()),
        };
        self.operate(|state| {
            host::log(format!("Detecting {pins}-pin SPI device..."));
            let db = state.db.clone().ok_or_else(|| {
                Fail::Soft("Chip database is still loading.".into())
            })?;
            let found = {
                let handle = state.handle.as_mut().expect("handle");
                spi_autodetect_and_lookup(handle, &db, id_type).map_err(classify)?
            };
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
            host::log(&message);
            Ok(SpiReport {
                jedec_id: format!("{:#010x}", found.jedec_id),
                matches,
            })
        })
    }

    fn operate<T: Serialize>(&mut self, f: impl FnOnce(&mut State) -> Result<T, Fail>) -> Result<String, String> {
        if self.handle.is_none() {
            return Err("No programmer connected. Rescan and try again.".into());
        }
        let result = f(self);
        if matches!(result, Err(Fail::Fatal(_))) {
            self.handle = None;
            self.last = None;
        }
        match result {
            Ok(value) => Ok(json(&value)),
            Err(Fail::Soft(message) | Fail::Fatal(message)) => Err(message),
        }
    }

    fn prepare(&mut self) -> bool {
        if let Some(handle) = self.handle.as_mut() {
            handle.icsp = self.icsp;
        }
        !self.skip_id
    }

    fn lookup(&self, name: &str) -> Result<Device, Fail> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| Fail::Soft("Chip database is still loading.".into()))?;
        let model = self
            .handle
            .as_ref()
            .ok_or_else(|| Fail::Soft("No programmer connected. Rescan and try again.".into()))?
            .info
            .model;
        find_device(db, name, model).map_err(|err| Fail::Soft(err.to_string()))
    }

    fn buffer_image(&self, memory: usize) -> Result<Vec<u8>, String> {
        if self.buffer.len() != memory {
            return Err(format!(
                "Buffer is {} bytes, but this memory is {memory} bytes.",
                self.buffer.len()
            ));
        }
        Ok(self.buffer.clone())
    }

    fn ensure_index(&mut self, model: ProgrammerModel) -> Result<(), String> {
        if self.index.contains_key(&model) {
            return Ok(());
        }
        let db = self.db.as_ref().ok_or("Chip database is still loading.")?;
        let items = list_devices_for_model(db, None, model).map_err(|err| err.to_string())?;
        self.index.insert(model, items);
        Ok(())
    }

    fn status(&self, busy: bool) -> ProgrammerStatus {
        match &self.last {
            Some(info) if self.handle.is_some() => ProgrammerStatus {
                connected: true,
                busy,
                model: Some(info.model.clone()),
                firmware: Some(info.firmware.clone()),
                serial_number: Some(info.serial_number.clone()),
                message: format!("Connected to {}, firmware {}.", info.model, info.firmware),
                warning: self
                    .handle
                    .as_ref()
                    .zip(self.db.as_ref())
                    .and_then(|(handle, db)| algorithm_warning(handle.info.model, db)),
            },
            _ => disconnected("No programmer found."),
        }
    }
}

fn disconnected(message: &str) -> ProgrammerStatus {
    ProgrammerStatus {
        connected: false,
        busy: false,
        model: None,
        firmware: None,
        serial_number: None,
        message: message.to_string(),
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

fn classify(err: MiniproError) -> Fail {
    let message = err.to_string();
    if err.is_usb_communication_error() {
        Fail::Fatal(message)
    } else {
        Fail::Soft(message)
    }
}

fn json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|err| format!("{{\"message\":\"{err}\"}}"))
}

fn parse_model(model: &str) -> Result<ProgrammerModel, String> {
    model.parse()
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
        kind: kind_name(device.chip_type).to_string(),
        fuses: fuse_fields(device),
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

fn crc_hex(data: &[u8]) -> String {
    format!(
        "{:08x}",
        crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC).checksum(data)
    )
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

fn decode(format: &str, bytes: &[u8], memory: usize, blank: u8) -> Result<Vec<u8>, String> {
    match format {
        "ihex" => ihex::read_from(Cursor::new(bytes), memory, blank).map_err(|err| err.to_string()),
        "srec" => srec::read_from(Cursor::new(bytes), memory, blank).map_err(|err| err.to_string()),
        "jedec" => jedec::read_from(Cursor::new(bytes), memory).map_err(|err| err.to_string()),
        _ => Ok(bytes.to_vec()),
    }
}

fn encode(filename: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    match format_for(Path::new(filename)) {
        "ihex" => ihex::write_to(&mut out, data).map_err(|err| err.to_string())?,
        "srec" => srec::write_to(&mut out, data).map_err(|err| err.to_string())?,
        "jedec" => jedec::write_to(&mut out, data, None).map_err(|err| err.to_string())?,
        _ => out.extend_from_slice(data),
    }
    Ok(out)
}

fn analyze(data: &[u8]) -> DumpReport {
    let mut lines = Vec::new();
    if data.is_empty() {
        lines.push("The buffer is empty.".into());
    } else if let Some(fill) = uniform_fill(data) {
        lines.push(format!(
            "Every byte is {fill:02X}. An all-FF read is a blank EPROM or data pins that are not connected. A blank 2764 (8192 x FF) is CRC32 b4293435, and mame -romident will match that image to blank graphics ROMs."
        ));
    } else {
        let (stuck_high, stuck_low) = stuck_bits(data);
        if stuck_high != 0 || stuck_low != 0 {
            lines.push(format!(
                "Stuck bits:{}{}.",
                bit_list(" always 1", stuck_high),
                bit_list(" always 0", stuck_low)
            ));
        }
        if data.len() % 2 == 0 && data[..data.len() / 2] == data[data.len() / 2..] {
            lines.push("The two halves of the dump are identical.".into());
        }
        if lines.is_empty() {
            lines.push("No blank fill, stuck bits, or repeated halves.".into());
        }
    }
    DumpReport {
        crc32: crc_hex(data),
        sha1: sha1_hex(data),
        length: data.len() as u64,
        lines,
    }
}

fn compare(dump: &[u8], other: &[u8], other_name: &str) -> Vec<String> {
    if dump.is_empty() {
        return vec!["The buffer is empty.".into()];
    }
    if other.is_empty() {
        return vec![format!("{other_name} is empty.")];
    }
    let mut lines = Vec::new();
    if dump.len() != other.len() {
        lines.push(format!(
            "Buffer is {} bytes; {other_name} is {} bytes.",
            dump.len(),
            other.len()
        ));
    }
    let n = dump.len().min(other.len());
    let same = dump[..n]
        .iter()
        .zip(other[..n].iter())
        .filter(|(left, right)| left == right)
        .count();
    if dump.len() == other.len() && same == n {
        lines.push(format!("Identical to {other_name}."));
        return lines;
    }
    let pct = if n == 0 {
        0.0
    } else {
        (same as f64) * 100.0 / (n as f64)
    };
    lines.push(format!(
        "{same} of {n} bytes match {other_name} ({pct:.2}%)."
    ));
    let mut diff_bits = 0u8;
    for (left, right) in dump[..n].iter().zip(other[..n].iter()) {
        diff_bits |= left ^ right;
    }
    if diff_bits != 0 {
        lines.push(format!("Differing bits:{}.", bit_list("", diff_bits)));
    }
    lines
}

fn uniform_fill(data: &[u8]) -> Option<u8> {
    let first = *data.first()?;
    if data.iter().all(|byte| *byte == first) {
        Some(first)
    } else {
        None
    }
}

fn stuck_bits(data: &[u8]) -> (u8, u8) {
    let mut always_one = 0xffu8;
    let mut seen = 0u8;
    for byte in data {
        always_one &= byte;
        seen |= byte;
    }
    (always_one, !seen)
}

fn bit_list(label: &str, bits: u8) -> String {
    if bits == 0 {
        return String::new();
    }
    let list = (0..8)
        .filter(|bit| bits & (1 << bit) != 0)
        .map(|bit| bit.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!("{label} {list}")
}

fn sha1_hex(data: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha1::digest(data);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}
