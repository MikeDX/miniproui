mod host;
mod session;

use std::sync::Mutex;

use serde_json::Value;
use session::State;
use wasm_bindgen::prelude::*;

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with_state<T>(f: impl FnOnce(&mut State) -> Result<T, String>) -> Result<T, JsValue> {
    let mut guard = STATE.lock().unwrap_or_else(|poison| poison.into_inner());
    let state = guard.get_or_insert_with(State::new);
    f(state).map_err(|err| JsValue::from_str(&err))
}

#[wasm_bindgen]
pub fn usb_memory() -> JsValue {
    wasm_bindgen::memory()
}

#[wasm_bindgen]
pub fn mount_databases(infoic: &str, logicic: &str, algorithms: Option<String>) -> Result<(), JsValue> {
    with_state(|state| {
        state.mount(infoic, logicic, algorithms.as_deref());
        Ok(())
    })
}

#[wasm_bindgen]
pub fn load_file_bytes(
    name: &str,
    filename: &str,
    page: u8,
    model: &str,
    bytes: &[u8],
) -> Result<String, JsValue> {
    with_state(|state| state.load_bytes(name, filename, page, model, bytes))
}

#[wasm_bindgen]
pub fn compare_bytes(filename: &str, bytes: &[u8]) -> Result<String, JsValue> {
    with_state(|state| state.compare(filename, bytes))
}

#[wasm_bindgen]
pub fn encode_buffer(filename: &str) -> Result<Vec<u8>, JsValue> {
    with_state(|state| state.encode(filename))
}

#[wasm_bindgen]
pub fn dispatch(method: &str, args_json: &str) -> Result<String, JsValue> {
    let args: Value = serde_json::from_str(args_json).unwrap_or(Value::Null);
    with_state(|state| call(state, method, &args))
}

fn call(state: &mut State, method: &str, args: &Value) -> Result<String, String> {
    match method {
        "programmer_status" => state.programmer_status(
            bool_arg(args, "force")?,
            u16_arg(args, "vid").unwrap_or(0),
            u16_arg(args, "pid").unwrap_or(0),
        ),
        "search_devices" => state.search(str_arg(args, "query")?, str_arg(args, "model")?),
        "device_info" => state.device_info(str_arg(args, "name")?, str_arg(args, "model")?),
        "hex_window" => state.hex_window(u64_arg(args, "offset")?, u32_arg(args, "length")?),
        "read_memory" => state.read_memory(str_arg(args, "device")?, u8_arg(args, "page")?),
        "write_buffer" => state.write_buffer(
            str_arg(args, "device")?,
            u8_arg(args, "page")?,
            bool_arg(args, "confirm")?,
        ),
        "verify_buffer" => state.verify_buffer(str_arg(args, "device")?, u8_arg(args, "page")?),
        "erase" => state.erase(str_arg(args, "device")?, bool_arg(args, "confirm")?),
        "blank_check" => state.blank_check(str_arg(args, "device")?),
        "read_id" => state.read_id(str_arg(args, "device")?),
        "set_options" => state.set_options(bool_arg(args, "skipId")?, str_arg(args, "icsp")?),
        "patch_buffer" => {
            let bytes = args
                .get("bytes")
                .and_then(Value::as_array)
                .ok_or("missing bytes")?
                .iter()
                .map(|value| {
                    value
                        .as_u64()
                        .and_then(|byte| u8::try_from(byte).ok())
                        .ok_or_else(|| "patch byte is out of range".to_string())
                })
                .collect::<Result<Vec<u8>, String>>()?;
            state.patch(u64_arg(args, "offset")?, &bytes)
        }
        "read_config" => state.read_config(str_arg(args, "device")?),
        "write_config" => {
            let fuses = args
                .get("fuses")
                .and_then(Value::as_array)
                .ok_or("missing fuses")?
                .iter()
                .map(|value| {
                    let name = value
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or("fuse is missing a name")?
                        .to_string();
                    let raw = value.get("value").and_then(Value::as_u64).ok_or("fuse is missing a value")?;
                    let value = u16::try_from(raw).map_err(|_| format!("fuse {name} is out of range"))?;
                    Ok((name, value))
                })
                .collect::<Result<Vec<_>, String>>()?;
            state.write_config(str_arg(args, "device")?, &fuses, bool_arg(args, "confirm")?)
        }
        "logic_test" => state.logic_test(str_arg(args, "device")?),
        "pin_check" => state.pin_check(str_arg(args, "device")?),
        "hardware_test" => state.hardware_test(),
        "spi_detect" => state.spi_detect(u8_arg(args, "pins")?),
        "buffer_report" => state.buffer_report(),
        "rom_ident" => Err(
            "ROM identification runs the local mame tool. Use the desktop app for that.".into(),
        ),
        "update_firmware" => Err(
            "Firmware update resets the USB device, and WebUSB cannot claim it again afterwards. Use the desktop app for firmware updates.".into(),
        ),
        "load_file" | "save_buffer" | "verify_file" | "write_file" | "compare_buffer" => Err(
            "The browser build reads and writes files in the page, not on a filesystem path.".into(),
        ),
        other => Err(format!("Unknown command {other}.")),
    }
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing {key}"))
}

fn bool_arg(args: &Value, key: &str) -> Result<bool, String> {
    args.get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("missing {key}"))
}

fn u64_arg(args: &Value, key: &str) -> Result<u64, String> {
    args.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing {key}"))
}

fn u32_arg(args: &Value, key: &str) -> Result<u32, String> {
    u64_arg(args, key)?
        .try_into()
        .map_err(|_| format!("{key} is out of range"))
}

fn u16_arg(args: &Value, key: &str) -> Result<u16, String> {
    u64_arg(args, key)?
        .try_into()
        .map_err(|_| format!("{key} is out of range"))
}

fn u8_arg(args: &Value, key: &str) -> Result<u8, String> {
    u64_arg(args, key)?
        .try_into()
        .map_err(|_| format!("{key} is out of range"))
}
