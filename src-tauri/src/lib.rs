// Copyright (C) 2026 MikeDX https://github.com/MikeDX/miniui
// Protocol based on minipro, created by Valentin Dudouyt in 2014.
// Development of that C project is coordinated by David Griffith.
// SPDX-License-Identifier: GPL-3.0-or-later

mod roms;
mod session;

use roms::{DumpReport, RomIdentReport};
use session::{
    AppState, DeviceInfo, FileReport, FuseReading, FuseWrite, HexWindow, LogicReport, OpReport,
    PinReport, ProgrammerStatusDto, SearchResponse, SpiReport,
};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Clone, serde::Serialize)]
struct LogEvent {
    message: String,
}

fn log_line(app: &AppHandle, message: impl AsRef<str>) {
    let message = message.as_ref().to_string();
    eprintln!("miniproui: {message}");
    let _ = app.emit("log", LogEvent { message });
}

fn report_error(app: &AppHandle, result: Result<OpReport, String>) -> Result<OpReport, String> {
    if let Err(ref err) = result {
        log_line(app, err);
    }
    result
}

#[tauri::command]
async fn programmer_status(
    app: AppHandle,
    state: State<'_, AppState>,
    force: bool,
) -> Result<ProgrammerStatusDto, String> {
    let state = state.inner().clone();
    let status = tauri::async_runtime::spawn_blocking(move || state.refresh_programmer(force))
        .await
        .map_err(|err| err.to_string())?;
    if force || !status.connected {
        log_line(&app, &status.message);
    }
    if let Some(warning) = &status.warning {
        log_line(&app, warning);
    }
    Ok(status)
}

#[tauri::command]
async fn search_devices(
    state: State<'_, AppState>,
    query: String,
    model: String,
) -> Result<SearchResponse, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.search(&query, &model))
        .await
        .map_err(|err| err.to_string())?
}

#[tauri::command]
async fn device_info(
    state: State<'_, AppState>,
    name: String,
    model: String,
) -> Result<DeviceInfo, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.device_info(&name, &model))
        .await
        .map_err(|err| err.to_string())?
}

#[tauri::command]
async fn load_file(
    state: State<'_, AppState>,
    path: String,
    device: String,
    page: u8,
    model: String,
) -> Result<FileReport, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let device = if device.is_empty() {
            None
        } else {
            Some(device)
        };
        state.load_file(
            PathBuf::from(path).as_path(),
            device.as_deref(),
            page,
            &model,
        )
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
async fn hex_window(
    state: State<'_, AppState>,
    offset: u64,
    length: u32,
) -> Result<HexWindow, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.hex_window(offset, length))
        .await
        .map_err(|err| err.to_string())?
}

#[tauri::command]
async fn save_buffer(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    let state = state.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        state.save_buffer(PathBuf::from(&path).as_path())
    })
    .await
    .map_err(|err| err.to_string())?;
    match result {
        Ok(message) => {
            log_line(&app, &message);
            Ok(message)
        }
        Err(err) => {
            log_line(&app, &err);
            Err(err)
        }
    }
}

#[tauri::command]
async fn read_memory(
    app: AppHandle,
    state: State<'_, AppState>,
    device: String,
    page: u8,
) -> Result<OpReport, String> {
    let state = state.inner().clone();
    let app2 = app.clone();
    let result =
        tauri::async_runtime::spawn_blocking(move || state.read_memory(&app2, &device, page))
            .await
            .map_err(|err| err.to_string())?;
    report_error(&app, result)
}

#[tauri::command]
async fn write_file(
    app: AppHandle,
    state: State<'_, AppState>,
    device: String,
    page: u8,
    path: String,
    confirm: bool,
) -> Result<OpReport, String> {
    let state = state.inner().clone();
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        state.write_memory(
            &app2,
            &device,
            page,
            PathBuf::from(&path).as_path(),
            confirm,
        )
    })
    .await
    .map_err(|err| err.to_string())?;
    report_error(&app, result)
}

#[tauri::command]
async fn verify_file(
    app: AppHandle,
    state: State<'_, AppState>,
    device: String,
    page: u8,
    path: String,
) -> Result<OpReport, String> {
    let state = state.inner().clone();
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        state.verify_memory(&app2, &device, page, PathBuf::from(&path).as_path())
    })
    .await
    .map_err(|err| err.to_string())?;
    report_error(&app, result)
}

#[tauri::command]
async fn erase(
    app: AppHandle,
    state: State<'_, AppState>,
    device: String,
    confirm: bool,
) -> Result<OpReport, String> {
    let state = state.inner().clone();
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || state.erase(&app2, &device, confirm))
        .await
        .map_err(|err| err.to_string())?;
    report_error(&app, result)
}

#[tauri::command]
async fn blank_check(
    app: AppHandle,
    state: State<'_, AppState>,
    device: String,
) -> Result<OpReport, String> {
    let state = state.inner().clone();
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || state.blank_check(&app2, &device))
        .await
        .map_err(|err| err.to_string())?;
    report_error(&app, result)
}

#[tauri::command]
async fn read_id(
    app: AppHandle,
    state: State<'_, AppState>,
    device: String,
) -> Result<OpReport, String> {
    let state = state.inner().clone();
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || state.read_id(&app2, &device))
        .await
        .map_err(|err| err.to_string())?;
    report_error(&app, result)
}

#[tauri::command]
async fn set_options(
    state: State<'_, AppState>,
    skip_id: bool,
    icsp: String,
) -> Result<(), String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.set_options(skip_id, &icsp))
        .await
        .map_err(|err| err.to_string())?
}

#[tauri::command]
async fn patch_buffer(
    state: State<'_, AppState>,
    offset: u64,
    bytes: Vec<u8>,
) -> Result<String, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.patch_buffer(offset, &bytes))
        .await
        .map_err(|err| err.to_string())?
}

#[tauri::command]
async fn write_buffer(
    app: AppHandle,
    state: State<'_, AppState>,
    device: String,
    page: u8,
    confirm: bool,
) -> Result<OpReport, String> {
    let state = state.inner().clone();
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        state.write_buffer(&app2, &device, page, confirm)
    })
    .await
    .map_err(|err| err.to_string())?;
    report_error(&app, result)
}

#[tauri::command]
async fn verify_buffer(
    app: AppHandle,
    state: State<'_, AppState>,
    device: String,
    page: u8,
) -> Result<OpReport, String> {
    let state = state.inner().clone();
    let app2 = app.clone();
    let result =
        tauri::async_runtime::spawn_blocking(move || state.verify_buffer(&app2, &device, page))
            .await
            .map_err(|err| err.to_string())?;
    report_error(&app, result)
}

#[tauri::command]
async fn read_config(
    app: AppHandle,
    state: State<'_, AppState>,
    device: String,
) -> Result<Vec<FuseReading>, String> {
    let state = state.inner().clone();
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || state.read_config(&app2, &device))
        .await
        .map_err(|err| err.to_string())?;
    if let Err(ref err) = result {
        log_line(&app, err);
    }
    result
}

#[tauri::command]
async fn write_config(
    app: AppHandle,
    state: State<'_, AppState>,
    device: String,
    fuses: Vec<FuseWrite>,
    confirm: bool,
) -> Result<OpReport, String> {
    let state = state.inner().clone();
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        state.write_config(&app2, &device, &fuses, confirm)
    })
    .await
    .map_err(|err| err.to_string())?;
    report_error(&app, result)
}

#[tauri::command]
async fn logic_test(
    app: AppHandle,
    state: State<'_, AppState>,
    device: String,
) -> Result<LogicReport, String> {
    let state = state.inner().clone();
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || state.logic_test(&app2, &device))
        .await
        .map_err(|err| err.to_string())?;
    if let Err(ref err) = result {
        log_line(&app, err);
    }
    result
}

#[tauri::command]
async fn pin_check(
    app: AppHandle,
    state: State<'_, AppState>,
    device: String,
) -> Result<PinReport, String> {
    let state = state.inner().clone();
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || state.pin_check(&app2, &device))
        .await
        .map_err(|err| err.to_string())?;
    if let Err(ref err) = result {
        log_line(&app, err);
    }
    result
}

#[tauri::command]
async fn hardware_test(app: AppHandle, state: State<'_, AppState>) -> Result<OpReport, String> {
    let state = state.inner().clone();
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || state.hardware_test(&app2))
        .await
        .map_err(|err| err.to_string())?;
    report_error(&app, result)
}

#[tauri::command]
async fn spi_detect(
    app: AppHandle,
    state: State<'_, AppState>,
    pins: u8,
) -> Result<SpiReport, String> {
    let state = state.inner().clone();
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || state.spi_detect(&app2, pins))
        .await
        .map_err(|err| err.to_string())?;
    if let Err(ref err) = result {
        log_line(&app, err);
    }
    result
}

#[tauri::command]
async fn buffer_report(state: State<'_, AppState>) -> Result<DumpReport, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.buffer_report())
        .await
        .map_err(|err| err.to_string())?
}

#[tauri::command]
async fn rom_ident(state: State<'_, AppState>) -> Result<RomIdentReport, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.rom_ident())
        .await
        .map_err(|err| err.to_string())?
}

#[tauri::command]
async fn compare_buffer(state: State<'_, AppState>, path: String) -> Result<DumpReport, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.compare_buffer(PathBuf::from(path)))
        .await
        .map_err(|err| err.to_string())?
}

#[tauri::command]
async fn update_firmware(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    confirm: bool,
) -> Result<OpReport, String> {
    let state = state.inner().clone();
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        state.update_firmware(&app2, PathBuf::from(&path).as_path(), confirm)
    })
    .await
    .map_err(|err| err.to_string())?;
    report_error(&app, result)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let state = AppState::new().expect("chip database");
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            programmer_status,
            search_devices,
            device_info,
            load_file,
            hex_window,
            save_buffer,
            read_memory,
            write_file,
            verify_file,
            erase,
            blank_check,
            read_id,
            set_options,
            patch_buffer,
            write_buffer,
            verify_buffer,
            read_config,
            write_config,
            logic_test,
            pin_check,
            hardware_test,
            spi_detect,
            update_firmware,
            buffer_report,
            rom_ident,
            compare_buffer
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
