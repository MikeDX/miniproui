mod session;

use session::{
    AppState, DeviceInfo, FileReport, HexWindow, OpReport, ProgrammerStatusDto, SearchResponse,
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
            read_id
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
