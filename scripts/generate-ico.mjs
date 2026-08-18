#!/usr/bin/env node
// Generate the Windows .ico file. Windows accepts PNG bytes embedded in ICO
// since Vista, so we just wrap the 32x32 PNG.
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const iconsDir = join(__dirname, "..", "src-tauri", "icons");

const png = readFileSync(join(iconsDir, "32x32.png"));
// 32x32 fits in the byte field (0 means 256).
const widthByte = 32 === 256 ? 0 : 32;
const heightByte = 32 === 256 ? 0 : 32;

const header = Buffer.alloc(6);
header.writeUInt16LE(0, 0); // reserved
header.writeUInt16LE(1, 2); // type: ICO
header.writeUInt16LE(1, 4); // count

const entry = Buffer.alloc(16);
entry[0] = widthByte;
entry[1] = heightByte;
entry[2] = 0; // color count
entry[3] = 0; // reserved
entry.writeUInt16LE(1, 4); // planes
entry.writeUInt16LE(32, 6); // bits per pixel
entry.writeUInt32LE(png.length, 8); // image size
entry.writeUInt32LE(6 + 16, 12); // offset to image data

writeFileSync(join(iconsDir, "icon.ico"), Buffer.concat([header, entry, png]));
console.log("wrote icon.ico");
