# miniproui

A desktop app for XGecu TL866A/CS, TL866II+, T48, T56, and T76 programmers. It talks to the programmer over USB through [minipro-core](https://github.com/MikeDX/minipro-rs) and does not shell out to the C `minipro` binary.

The first version can detect the programmer, search the chip database, read, write, verify, erase, blank-check, and read a chip ID. Reads and opened files show up in a hex view, with a live log of each operation.

This program is GPL-3.0-or-later, the same license as `minipro` and `minipro-core`.

## Develop

On macOS, install Rust and Node, then:

```sh
git submodule update --init
npm install
npm run tauri dev
```

The Mac App Sandbox stays off so the app can open the programmer. Quit the C `minipro` tool before connecting; only one program can hold the USB device.

Search and file loading work with no programmer attached. Pick the TL866II+ database or the TL866A database in the sidebar. Plug in a programmer to run read, write, verify, erase, blank-check, and read ID.

## Chromium WebUSB

`npm run dev:web` serves the same UI in Chrome or Edge and talks to the programmer with WebUSB. The chip protocol still runs in Rust, compiled to WebAssembly. The page asks for the device when you press Rescan. Firefox and Safari do not implement WebUSB.

Quit the desktop app first. Only one program can claim the programmer. Firmware update stays in the desktop app, because a firmware update reboots the USB device. ROM identification in the web build looks up the dump's SHA-1 in static MAME shards under `/romdb/`, built locally with `npm run romdb -- --mame /path/to/mame`. The desktop app still shells out to `mame -romident`.

The dev server sends the cross-origin isolation headers WebAssembly needs for the USB transfers. A static host has to send the same headers:

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

`npm run build:web` writes that site to `build/`.

T56 and T76 chips that need an FPGA bitstream look for `algorithm.xml` next to the bundled database, in Homebrew's minipro share directory, or in the Cellar install. That file is not included here.
