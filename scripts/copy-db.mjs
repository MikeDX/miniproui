import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import path from "node:path";

const from = path.resolve("third_party/minipro-rs/data");
const to = path.resolve("build/db");
mkdirSync(to, { recursive: true });

for (const name of ["infoic.xml", "logicic.xml", "algorithm.xml"]) {
  const source = path.join(from, name);
  if (!existsSync(source)) continue;
  copyFileSync(source, path.join(to, name));
}
