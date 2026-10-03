# miniproui

A desktop app for XGecu TL866A/CS, TL866II+, T48, T56, and T76 programmers. It talks to the programmer over USB through [minipro-core](https://gitlab.com/arcturus8081/minipro-rs) and does not shell out to the C `minipro` binary.

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

T56 and T76 chips that need an FPGA bitstream look for `algorithm.xml` next to the bundled database, in Homebrew's minipro share directory, or in the Cellar install. That file is not included here.
