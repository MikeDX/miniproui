// @ts-nocheck — this config uses Node builtins without @types/node.
// Copyright (C) 2026 MikeDX https://github.com/MikeDX/miniui
// Protocol based on minipro, created by Valentin Dudouyt in 2014.
// Development of that C project is coordinated by David Griffith.
// SPDX-License-Identifier: GPL-3.0-or-later

import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const host = process.env.TAURI_DEV_HOST;
const webusb = process.env.MINIPRO_WEBUSB === "1";
const databaseDir = path.resolve("third_party/minipro-rs/data");
const isolation = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

function serveDatabase(req, res, next) {
  const name = String(req.url || "")
    .split("?")[0]
    .replace(/^\//, "");
  if (!["infoic.xml", "logicic.xml", "algorithm.xml"].includes(name)) {
    next();
    return;
  }
  const file = path.join(databaseDir, name);
  if (!fs.existsSync(file)) {
    next();
    return;
  }
  res.setHeader("Content-Type", "application/xml");
  res.setHeader("Cross-Origin-Resource-Policy", "same-origin");
  fs.createReadStream(file).pipe(res);
}

function serveRomdb(req, res, next) {
  const name = String(req.url || "")
    .split("?")[0]
    .replace(/^\//, "");
  if (!/^([0-9a-f]{3}|meta)\.json$/.test(name)) {
    next();
    return;
  }
  const file = path.join("static-romdb", name);
  if (!fs.existsSync(file)) {
    next();
    return;
  }
  res.setHeader("Content-Type", "application/json");
  res.setHeader("Cross-Origin-Resource-Policy", "same-origin");
  fs.createReadStream(file).pipe(res);
}

function isolationHeaders(_req, res, next) {
  if (!webusb) {
    next();
    return;
  }
  res.setHeader("Cross-Origin-Opener-Policy", "same-origin");
  res.setHeader("Cross-Origin-Embedder-Policy", "require-corp");
  next();
}

function miniproDatabase() {
  return {
    name: "minipro-database",
    enforce: "pre",
    configureServer(server) {
      server.middlewares.use(isolationHeaders);
      server.middlewares.use("/db", serveDatabase);
      server.middlewares.use("/romdb", serveRomdb);
    },
    configurePreviewServer(server) {
      server.middlewares.use(isolationHeaders);
      server.middlewares.use("/db", serveDatabase);
      server.middlewares.use("/romdb", serveRomdb);
    },
  };
}

export default defineConfig(() => ({
  plugins: [sveltekit(), miniproDatabase()],
  resolve: {
    alias: webusb
      ? []
      : [
          {
            find: /webusb\/client(?:\.ts)?$/,
            replacement: path.resolve("src/lib/webusb/stub.ts"),
          },
        ],
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || "127.0.0.1",
    headers: webusb ? isolation : undefined,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**", "**/third_party/**", "**/web-core/target/**", "**/static-romdb/**", "**/static-romdb.tmp/**"],
    },
  },
  preview: {
    headers: webusb ? isolation : undefined,
  },
  worker: {
    format: "es",
  },
}));
