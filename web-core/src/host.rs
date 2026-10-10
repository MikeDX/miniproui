// Copyright (C) 2026 MikeDX https://github.com/MikeDX/miniui
// Protocol based on minipro, created by Valentin Dudouyt in 2014.
// Development of that C project is coordinated by David Griffith.
// SPDX-License-Identifier: GPL-3.0-or-later

//! JavaScript hooks for the WebUSB transport.
//!
//! The functions are synchronous because the chip protocol is synchronous.
//! They block the worker with `Atomics.wait` while the page performs the
//! WebUSB transfer.

use minipro_core::error::{MiniproError, Result};
use minipro_core::usb::HostBulk;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = globalThis)]
    fn miniproHostLog(message: &str);

    #[wasm_bindgen(js_namespace = globalThis)]
    fn miniproHostProgress(done: f64, total: f64);

    #[wasm_bindgen(js_namespace = globalThis)]
    fn miniproHostOut(ep: u32, ptr: u32, len: u32) -> i32;

    #[wasm_bindgen(js_namespace = globalThis)]
    fn miniproHostIn(ep: u32, ptr: u32, len: u32) -> i32;

    #[wasm_bindgen(js_namespace = globalThis)]
    fn miniproHostOutPair(
        ep_a: u32,
        ptr_a: u32,
        len_a: u32,
        ep_b: u32,
        ptr_b: u32,
        len_b: u32,
    ) -> i32;

    #[wasm_bindgen(js_namespace = globalThis)]
    fn miniproHostInPair(
        ep_a: u32,
        ptr_a: u32,
        len_a: u32,
        ep_b: u32,
        ptr_b: u32,
        len_b: u32,
    ) -> i32;

    #[wasm_bindgen(js_namespace = globalThis)]
    fn miniproHostError() -> String;
}

pub fn log(message: impl AsRef<str>) {
    miniproHostLog(message.as_ref());
}

pub fn progress(done: usize, total: usize) {
    miniproHostProgress(done as f64, total as f64);
}

pub struct JsHost;

fn fail() -> MiniproError {
    MiniproError::Protocol(miniproHostError())
}

impl HostBulk for JsHost {
    fn transfer_out(&self, endpoint: u8, data: &[u8]) -> Result<()> {
        let rc = miniproHostOut(endpoint as u32, data.as_ptr() as u32, data.len() as u32);
        if rc < 0 { Err(fail()) } else { Ok(()) }
    }

    fn transfer_in(&self, endpoint: u8, length: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; length];
        let rc = miniproHostIn(endpoint as u32, buf.as_mut_ptr() as u32, length as u32);
        if rc < 0 {
            return Err(fail());
        }
        let n = rc as usize;
        if n > buf.len() {
            return Err(MiniproError::Protocol(format!(
                "USB error: host returned {n} bytes into a {length}-byte buffer"
            )));
        }
        buf.truncate(n);
        Ok(buf)
    }

    fn transfer_out_pair(&self, ep_a: u8, a: &[u8], ep_b: u8, b: &[u8]) -> Result<()> {
        let rc = miniproHostOutPair(
            ep_a as u32,
            a.as_ptr() as u32,
            a.len() as u32,
            ep_b as u32,
            b.as_ptr() as u32,
            b.len() as u32,
        );
        if rc < 0 { Err(fail()) } else { Ok(()) }
    }

    fn transfer_in_pair(
        &self,
        ep_a: u8,
        len_a: usize,
        ep_b: u8,
        len_b: usize,
    ) -> Result<(Vec<u8>, Vec<u8>)> {
        let mut a = vec![0u8; len_a];
        let mut b = vec![0u8; len_b];
        let rc = miniproHostInPair(
            ep_a as u32,
            a.as_mut_ptr() as u32,
            len_a as u32,
            ep_b as u32,
            b.as_mut_ptr() as u32,
            len_b as u32,
        );
        if rc < 0 {
            return Err(fail());
        }
        Ok((a, b))
    }
}
