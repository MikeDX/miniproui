// Copyright (C) 2026 MikeDX https://github.com/MikeDX/miniui
// Protocol based on minipro, created by Valentin Dudouyt in 2014.
// Development of that C project is coordinated by David Griffith.
// SPDX-License-Identifier: GPL-3.0-or-later

import { cpSync, copyFileSync, existsSync, mkdirSync } from "node:fs";
import path from "node:path";

const from = path.resolve("third_party/minipro-rs/data");
const to = path.resolve("build/db");
mkdirSync(to, { recursive: true });

for (const name of ["infoic.xml", "logicic.xml", "algorithm.xml"]) {
  const source = path.join(from, name);
  if (!existsSync(source)) continue;
  copyFileSync(source, path.join(to, name));
}

const romdb = path.resolve("static-romdb");
if (existsSync(path.join(romdb, "meta.json"))) {
  cpSync(romdb, path.resolve("build/romdb"), { recursive: true });
}
