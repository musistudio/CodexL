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
const pwaDir = path.join(rootDir, "remote", "control-pwa");
const pwaIcon = path.join(pwaDir, "icon.png");
const pwaIconAssets = [
  { name: "favicon-32x32.png", size: 32 },
  { name: "apple-touch-icon.png", size: 180, opaque: true },
  { name: "icon-192.png", size: 192 },
  { name: "icon-512.png", size: 512 },
  { name: "icon-maskable-512.png", size: 512 },
];
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
syncPwaIcons(sourceIcon);

const tempDir = await mkdtemp(path.join(tmpdir(), "codexl-reicon-"));
try {
  const tempSource = path.join(tempDir, "source.png");
  const tempOutput = path.join(tempDir, "icons");
  copyFileSync(sourceIcon, tempSource);

  runTauriIcon(tempSource, tempOutput);
  syncGeneratedIcons(tempOutput);
  syncPwaIcons(sourceIcon);

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
    if (entry === "icon.png" || entry === "android" || entry === "ios") {
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

function syncPwaIcons(file) {
  copyFileSync(file, pwaIcon);

  const png = readPng(file);
  if (png.colorType !== 6 || png.bitDepth !== 8 || png.interlace !== 0) {
    fail("Source icon must be an 8-bit RGBA PNG before generating PWA icons.");
  }

  const rows = unfilterScanlines(zlib.inflateSync(png.imageData), png.width, png.height, 4);
  for (const asset of pwaIconAssets) {
    const source = asset.opaque ? cropVisibleRgbaRows(rows, png.width, png.height) : {
      height: png.height,
      rows,
      width: png.width,
    };
    const resizedRows = resizeRgbaRows(source.rows, source.width, source.height, asset.size);
    if (asset.opaque) {
      writeRgbPng(
        path.join(pwaDir, asset.name),
        asset.size,
        asset.size,
        opaqueRgbRowsWithSymbol(resizedRows, asset.size, asset.size),
      );
    } else {
      writeRgbaPng(path.join(pwaDir, asset.name), asset.size, asset.size, resizedRows);
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

function resizeRgbaRows(rows, sourceWidth, sourceHeight, targetSize) {
  const targetRows = [];
  const xScale = sourceWidth / targetSize;
  const yScale = sourceHeight / targetSize;

  for (let targetY = 0; targetY < targetSize; targetY += 1) {
    const output = Buffer.alloc(targetSize * 4);
    const sourceYStart = targetY * yScale;
    const sourceYEnd = (targetY + 1) * yScale;
    const sourceYMin = Math.floor(sourceYStart);
    const sourceYMax = Math.ceil(sourceYEnd);

    for (let targetX = 0; targetX < targetSize; targetX += 1) {
      const sourceXStart = targetX * xScale;
      const sourceXEnd = (targetX + 1) * xScale;
      const sourceXMin = Math.floor(sourceXStart);
      const sourceXMax = Math.ceil(sourceXEnd);
      let totalWeight = 0;
      let alpha = 0;
      let red = 0;
      let green = 0;
      let blue = 0;

      for (let sourceY = sourceYMin; sourceY < sourceYMax; sourceY += 1) {
        if (sourceY < 0 || sourceY >= sourceHeight) {
          continue;
        }

        const yWeight =
          Math.min(sourceY + 1, sourceYEnd) - Math.max(sourceY, sourceYStart);
        if (yWeight <= 0) {
          continue;
        }

        const sourceRow = rows[sourceY];
        for (let sourceX = sourceXMin; sourceX < sourceXMax; sourceX += 1) {
          if (sourceX < 0 || sourceX >= sourceWidth) {
            continue;
          }

          const xWeight =
            Math.min(sourceX + 1, sourceXEnd) - Math.max(sourceX, sourceXStart);
          if (xWeight <= 0) {
            continue;
          }

          const weight = xWeight * yWeight;
          const offset = sourceX * 4;
          const sourceAlpha = sourceRow[offset + 3] / 255;
          totalWeight += weight;
          alpha += sourceAlpha * weight;
          red += sourceRow[offset] * sourceAlpha * weight;
          green += sourceRow[offset + 1] * sourceAlpha * weight;
          blue += sourceRow[offset + 2] * sourceAlpha * weight;
        }
      }

      const outputOffset = targetX * 4;
      if (alpha > 0) {
        output[outputOffset] = clampByte(red / alpha);
        output[outputOffset + 1] = clampByte(green / alpha);
        output[outputOffset + 2] = clampByte(blue / alpha);
        output[outputOffset + 3] = clampByte((alpha / totalWeight) * 255);
      }
    }

    targetRows.push(output);
  }

  return targetRows;
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

function cropVisibleRgbaRows(rows, width, height) {
  let minX = width;
  let minY = height;
  let maxX = -1;
  let maxY = -1;

  for (let y = 0; y < height; y += 1) {
    const row = rows[y];
    for (let x = 0; x < width; x += 1) {
      if (row[x * 4 + 3] <= 8) {
        continue;
      }

      minX = Math.min(minX, x);
      minY = Math.min(minY, y);
      maxX = Math.max(maxX, x);
      maxY = Math.max(maxY, y);
    }
  }

  if (maxX < minX || maxY < minY) {
    return { height, rows, width };
  }

  const croppedWidth = maxX - minX + 1;
  const croppedHeight = maxY - minY + 1;
  const croppedRows = [];
  for (let y = minY; y <= maxY; y += 1) {
    croppedRows.push(Buffer.from(rows[y].subarray(minX * 4, (maxX + 1) * 4)));
  }

  return {
    height: croppedHeight,
    rows: croppedRows,
    width: croppedWidth,
  };
}

function writeRgbPng(file, width, height, rows) {
  const scanlines = rows.map((row) => Buffer.concat([Buffer.from([0]), row]));
  const outputChunks = [
    chunk(
      "IHDR",
      Buffer.concat([uint32(width), uint32(height), Buffer.from([8, 2, 0, 0, 0])]),
    ),
    chunk("IDAT", zlib.deflateSync(Buffer.concat(scanlines), { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ];

  writeFileSync(file, Buffer.concat([PNG_SIGNATURE, ...outputChunks]));
}

function opaqueRgbRowsWithSymbol(rows, width, height) {
  const top = [169, 139, 255];
  const center = [82, 101, 248];
  const bottom = [18, 86, 245];
  const outputRows = [];

  for (let y = 0; y < height; y += 1) {
    const row = rows[y];
    const output = Buffer.alloc(width * 3);
    for (let x = 0; x < width; x += 1) {
      const sourceOffset = x * 4;
      const targetOffset = x * 3;
      const alpha = whiteSymbolAlpha(row, sourceOffset, x, y, width, height);
      const background = iconBackgroundColor(x, y, width, height, top, center, bottom);
      const iconColor = [
        255 * alpha + background[0] * (1 - alpha),
        255 * alpha + background[1] * (1 - alpha),
        255 * alpha + background[2] * (1 - alpha),
      ];
      const maskAlpha = iosIconMaskAlpha(x, y, width, height);
      output[targetOffset] = clampByte(iconColor[0] * maskAlpha + 16 * (1 - maskAlpha));
      output[targetOffset + 1] = clampByte(iconColor[1] * maskAlpha + 17 * (1 - maskAlpha));
      output[targetOffset + 2] = clampByte(iconColor[2] * maskAlpha + 20 * (1 - maskAlpha));
    }
    outputRows.push(output);
  }

  return outputRows;
}

function whiteSymbolAlpha(row, offset, x, y, width, height) {
  if (x < width * 0.1 || x > width * 0.9 || y < height * 0.24 || y > height * 0.76) {
    return 0;
  }

  const sourceAlpha = row[offset + 3] / 255;
  if (sourceAlpha <= 0) {
    return 0;
  }

  const red = row[offset];
  const green = row[offset + 1];
  const blue = row[offset + 2];
  const min = Math.min(red, green, blue);
  const max = Math.max(red, green, blue);
  const brightness = clampUnit((min - 178) / 77);
  const neutrality = clampUnit((96 - (max - min)) / 96);

  return clampUnit(sourceAlpha * brightness * neutrality);
}

function iconBackgroundColor(x, y, width, height, top, center, bottom) {
  const vertical = height <= 1 ? 0 : y / (height - 1);
  const horizontal = width <= 1 ? 0 : x / (width - 1);
  const base = vertical < 0.5
    ? mixColor(top, center, vertical * 2)
    : mixColor(center, bottom, (vertical - 0.5) * 2);
  const highlight = (1 - vertical) * (1 - Math.abs(horizontal - 0.36) * 1.25);
  const depth = vertical * (0.72 + horizontal * 0.28);
  return [
    clampByte(base[0] + highlight * 18 - depth * 8),
    clampByte(base[1] + highlight * 14 - depth * 10),
    clampByte(base[2] + highlight * 20 + depth * 16),
  ];
}

function iosIconMaskAlpha(x, y, width, height) {
  const radius = Math.min(width, height) * 0.2237;
  const px = x + 0.5;
  const py = y + 0.5;
  const left = radius;
  const right = width - radius;
  const top = radius;
  const bottom = height - radius;
  const cornerX = px < left ? left : px > right ? right : px;
  const cornerY = py < top ? top : py > bottom ? bottom : py;
  const distance = Math.hypot(px - cornerX, py - cornerY);

  return clampUnit(radius + 0.5 - distance);
}

function averageVisibleColor(rows, width, startY, endY) {
  let red = 0;
  let green = 0;
  let blue = 0;
  let total = 0;

  for (let y = startY; y < endY && y < rows.length; y += 1) {
    const row = rows[y];
    for (let x = 0; x < width; x += 1) {
      const offset = x * 4;
      const alpha = row[offset + 3] / 255;
      if (alpha <= 0.2) {
        continue;
      }
      red += row[offset] * alpha;
      green += row[offset + 1] * alpha;
      blue += row[offset + 2] * alpha;
      total += alpha;
    }
  }

  if (total === 0) {
    return [88, 107, 244];
  }

  return [red / total, green / total, blue / total];
}

function mixColor(from, to, amount) {
  return [
    from[0] + (to[0] - from[0]) * amount,
    from[1] + (to[1] - from[1]) * amount,
    from[2] + (to[2] - from[2]) * amount,
  ];
}

function clampUnit(value) {
  return Math.max(0, Math.min(1, value));
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

function clampByte(value) {
  return Math.max(0, Math.min(255, Math.round(value)));
}

function printHelp() {
  console.log(`Usage: pnpm reicon

Regenerates platform icon resources from src-tauri/icons/icon.png.
The source icon.png is preserved and synced to remote/control-pwa with PWA icon sizes.`);
}

function fail(message) {
  console.error(message);
  process.exit(1);
}
