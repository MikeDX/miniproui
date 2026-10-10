// Copyright (C) 2026 MikeDX https://github.com/MikeDX/miniui
// Protocol based on minipro, created by Valentin Dudouyt in 2014.
// Development of that C project is coordinated by David Griffith.
// SPDX-License-Identifier: GPL-3.0-or-later

// Prevents an additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    miniproui_lib::run()
}
