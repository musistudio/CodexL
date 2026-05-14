#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import zlib from "node:zlib";

const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const iconsDir = path.join(rootDir, "src-tauri", "icons");
const sourceIcon = path.join(iconsDir, "icon.png");
const pwaIcon = path.join(rootDir, "remote", "control-pwa", "icon.png");
const tauriBin = path.join(
  rootDir,
  "node_modules",
  ".bin",
  process.platform === "win32" ? "tauri.cmd" : "tauri",
);

const { values } = parseArgs({
  allowPositionals: false,
  options: {
    help: { type: "boolean", short: "h" },
  },
});

if (values.help) {
  printHelp();
  process.exit(0);
}

if (!existsSync(sourceIcon)) {
  fail(`Missing source icon: ${path.relative(rootDir, sourceIcon)}`);
}

if (!existsSync(tauriBin)) {
  fail("Missing local Tauri CLI. Run your package manager install command first.");
}

mkdirSync(iconsDir, { recursive: true });
normalizeSourceIcon(sourceIcon);
removeEdgeBackground(sourceIcon);
copyFileSync(sourceIcon, pwaIcon);

const tempDir = await mkdtemp(path.join(tmpdir(), "codexl-reicon-"));
try {
  const tempSource = path.join(tempDir, "source.png");
  const tempOutput = path.join(tempDir, "icons");
  copyFileSync(sourceIcon, tempSource);

  runTauriIcon(tempSource, tempOutput);
  syncGeneratedIcons(tempOutput);
  copyFileSync(sourceIcon, pwaIcon);

  console.log("Regenerated icon resources from src-tauri/icons/icon.png");
} finally {
  rmSync(tempDir, { force: true, recursive: true });
}

function runTauriIcon(input, output) {
  const result = spawnSync(tauriBin, ["icon", input, "-o", output], {
    cwd: rootDir,
    stdio: "inherit",
  });

  if (result.error) {
    fail(`Failed to start Tauri icon generator: ${result.error.message}`);
  }

  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function syncGeneratedIcons(outputDir) {
  for (const entry of readdirSync(outputDir)) {
    if (entry === "icon.png") {
      continue;
    }

    const from = path.join(outputDir, entry);
    const to = path.join(iconsDir, entry);
    const stat = statSync(from);

    if (stat.isDirectory()) {
      rmSync(to, { force: true, recursive: true });
      cpSync(from, to, { force: true, recursive: true });
    } else if (stat.isFile()) {
      copyFileSync(from, to);
    }
  }
}

function normalizeSourceIcon(file) {
  const png = readPng(file);
  if (png.colorType === 6) {
    return;
  }

  if (png.colorType !== 2 || png.bitDepth !== 8 || png.interlace !== 0) {
    fail("Source icon must be an 8-bit RGB or RGBA PNG.");
  }

  const rows = unfilterScanlines(zlib.inflateSync(png.imageData), png.width, png.height, 3);
  const rgbaRows = [];
  for (const row of rows) {
    const output = Buffer.alloc((row.length / 3) * 4);
    let offset = 0;
    for (let i = 0; i < row.length; i += 3) {
      output[offset++] = row[i];
      output[offset++] = row[i + 1];
      output[offset++] = row[i + 2];
      output[offset++] = 255;
    }
    rgbaRows.push(output);
  }

  writeRgbaPng(file, png.width, png.height, rgbaRows);
}

function removeEdgeBackground(file) {
  const png = readPng(file);
  if (png.colorType !== 6 || png.bitDepth !== 8 || png.interlace !== 0) {
    return;
  }

  const rows = unfilterScanlines(zlib.inflateSync(png.imageData), png.width, png.height, 4);
  const seen = new Uint8Array(png.width * png.height);
  const queue = [];

  function push(x, y) {
    if (x < 0 || x >= png.width || y < 0 || y >= png.height) {
      return;
    }

    const point = y * png.width + x;
    if (seen[point]) {
      return;
    }

    const offset = x * 4;
    if (!isOpaqueCheckerboardPixel(rows[y], offset)) {
      return;
    }

    seen[point] = 1;
    queue.push(point);
  }

  for (let x = 0; x < png.width; x += 1) {
    push(x, 0);
    push(x, png.height - 1);
  }

  for (let y = 0; y < png.height; y += 1) {
    push(0, y);
    push(png.width - 1, y);
  }

  for (let index = 0; index < queue.length; index += 1) {
    const point = queue[index];
    const x = point % png.width;
    const y = (point - x) / png.width;
    push(x + 1, y);
    push(x - 1, y);
    push(x, y + 1);
    push(x, y - 1);
  }

  if (queue.length === 0) {
    return;
  }

  for (const point of queue) {
    const x = point % png.width;
    const y = (point - x) / png.width;
    rows[y][x * 4 + 3] = 0;
  }

  writeRgbaPng(file, png.width, png.height, rows);
}

function isOpaqueCheckerboardPixel(row, offset) {
  if (row[offset + 3] !== 255) {
    return false;
  }

  const red = row[offset];
  const green = row[offset + 1];
  const blue = row[offset + 2];
  const max = Math.max(red, green, blue);
  const min = Math.min(red, green, blue);

  return max >= 220 && max - min <= 14;
}

function writeRgbaPng(file, width, height, rows) {
  const scanlines = rows.map((row) => Buffer.concat([Buffer.from([0]), row]));
  const outputChunks = [
    chunk(
      "IHDR",
      Buffer.concat([uint32(width), uint32(height), Buffer.from([8, 6, 0, 0, 0])]),
    ),
    chunk("IDAT", zlib.deflateSync(Buffer.concat(scanlines), { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ];

  writeFileSync(file, Buffer.concat([PNG_SIGNATURE, ...outputChunks]));
}

function readPng(file) {
  const data = readFileSync(file);
  if (!data.subarray(0, PNG_SIGNATURE.length).equals(PNG_SIGNATURE)) {
    fail("Source icon is not a PNG file.");
  }

  let width = 0;
  let height = 0;
  let bitDepth = 0;
  let colorType = 0;
  let interlace = 0;
  const idat = [];
  let offset = PNG_SIGNATURE.length;

  while (offset < data.length) {
    const length = data.readUInt32BE(offset);
    const type = data.subarray(offset + 4, offset + 8).toString("ascii");
    const payload = data.subarray(offset + 8, offset + 8 + length);
    const expectedCrc = data.readUInt32BE(offset + 8 + length);
    const actualCrc = crc32(Buffer.concat([Buffer.from(type, "ascii"), payload]));
    if (expectedCrc !== actualCrc) {
      fail(`Source icon has an invalid ${type} chunk.`);
    }

    if (type === "IHDR") {
      width = payload.readUInt32BE(0);
      height = payload.readUInt32BE(4);
      bitDepth = payload[8];
      colorType = payload[9];
      interlace = payload[12];
    } else if (type === "IDAT") {
      idat.push(payload);
    } else if (type === "IEND") {
      break;
    }

    offset += 12 + length;
  }

  return {
    bitDepth,
    colorType,
    height,
    imageData: Buffer.concat(idat),
    interlace,
    width,
  };
}

function unfilterScanlines(raw, width, height, bytesPerPixel) {
  const stride = width * bytesPerPixel;
  const rows = [];
  let offset = 0;
  let previous = Buffer.alloc(stride);

  for (let rowIndex = 0; rowIndex < height; rowIndex += 1) {
    const filter = raw[offset++];
    const row = Buffer.from(raw.subarray(offset, offset + stride));
    offset += stride;

    if (row.length !== stride) {
      fail("Source icon has truncated PNG scanline data.");
    }

    for (let i = 0; i < stride; i += 1) {
      const left = i >= bytesPerPixel ? row[i - bytesPerPixel] : 0;
      const up = previous[i];
      const upLeft = i >= bytesPerPixel ? previous[i - bytesPerPixel] : 0;

      if (filter === 1) {
        row[i] = (row[i] + left) & 0xff;
      } else if (filter === 2) {
        row[i] = (row[i] + up) & 0xff;
      } else if (filter === 3) {
        row[i] = (row[i] + Math.floor((left + up) / 2)) & 0xff;
      } else if (filter === 4) {
        row[i] = (row[i] + paeth(left, up, upLeft)) & 0xff;
      } else if (filter !== 0) {
        fail(`Source icon has unsupported PNG filter ${filter}.`);
      }
    }

    rows.push(row);
    previous = row;
  }

  return rows;
}

function paeth(left, up, upLeft) {
  const predictor = left + up - upLeft;
  const leftDistance = Math.abs(predictor - left);
  const upDistance = Math.abs(predictor - up);
  const upLeftDistance = Math.abs(predictor - upLeft);

  if (leftDistance <= upDistance && leftDistance <= upLeftDistance) {
    return left;
  }

  return upDistance <= upLeftDistance ? up : upLeft;
}

function chunk(type, payload) {
  const typeBuffer = Buffer.from(type, "ascii");
  return Buffer.concat([uint32(payload.length), typeBuffer, payload, uint32(crc32(Buffer.concat([typeBuffer, payload])))]);
}

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc ^= byte;
    for (let i = 0; i < 8; i += 1) {
      crc = crc & 1 ? (crc >>> 1) ^ 0xedb88320 : crc >>> 1;
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function uint32(value) {
  const buffer = Buffer.alloc(4);
  buffer.writeUInt32BE(value);
  return buffer;
}

function printHelp() {
  console.log(`Usage: pnpm reicon

Regenerates platform icon resources from src-tauri/icons/icon.png.
The source icon.png is preserved and synced to remote/control-pwa/icon.png.`);
}

function fail(message) {
  console.error(message);
  process.exit(1);
}
