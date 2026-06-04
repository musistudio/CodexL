type QrSvgOptions = {
  moduleSize?: number;
  quietZone?: number;
  maxPixelSize?: number;
};

type DataCoord = {
  x: number;
  y: number;
};

type QrBlockGroup = {
  count: number;
  dataCodewords: number;
};

type QrVersionSpec = {
  alignmentPositions: number[];
  blocks: QrBlockGroup[];
  dataCodewords: number;
  ecCodewordsPerBlock: number;
  lengthBits: number;
  size: number;
  totalCodewords: number;
  version: number;
};

type MatrixState = {
  modules: (boolean | null)[][];
  reserved: boolean[][];
};

const BYTE_MODE = 0b0100;
const FORMAT_ECL_LOW = 0b01;
const PAD_CODEWORDS = [0xec, 0x11];
const QR_SPECS: QrVersionSpec[] = [
  qrSpec(7, [6, 22, 38], 20, [{ count: 2, dataCodewords: 78 }]),
  qrSpec(8, [6, 24, 42], 24, [{ count: 2, dataCodewords: 97 }]),
  qrSpec(9, [6, 26, 46], 30, [{ count: 2, dataCodewords: 116 }]),
  qrSpec(10, [6, 28, 50], 18, [
    { count: 2, dataCodewords: 68 },
    { count: 2, dataCodewords: 69 },
  ]),
  qrSpec(11, [6, 30, 54], 20, [{ count: 4, dataCodewords: 81 }]),
  qrSpec(12, [6, 32, 58], 24, [
    { count: 2, dataCodewords: 92 },
    { count: 2, dataCodewords: 93 },
  ]),
  qrSpec(13, [6, 34, 62], 26, [{ count: 4, dataCodewords: 107 }]),
  qrSpec(14, [6, 26, 46, 66], 30, [
    { count: 3, dataCodewords: 115 },
    { count: 1, dataCodewords: 116 },
  ]),
  qrSpec(15, [6, 26, 48, 70], 22, [
    { count: 5, dataCodewords: 87 },
    { count: 1, dataCodewords: 88 },
  ]),
  qrSpec(16, [6, 26, 50, 74], 24, [
    { count: 5, dataCodewords: 98 },
    { count: 1, dataCodewords: 99 },
  ]),
  qrSpec(17, [6, 30, 54, 78], 28, [
    { count: 1, dataCodewords: 107 },
    { count: 5, dataCodewords: 108 },
  ]),
  qrSpec(18, [6, 30, 56, 82], 30, [
    { count: 5, dataCodewords: 120 },
    { count: 1, dataCodewords: 121 },
  ]),
  qrSpec(19, [6, 30, 58, 86], 28, [
    { count: 3, dataCodewords: 113 },
    { count: 4, dataCodewords: 114 },
  ]),
  qrSpec(20, [6, 34, 62, 90], 28, [
    { count: 3, dataCodewords: 107 },
    { count: 5, dataCodewords: 108 },
  ]),
];
const GF_EXP = new Array<number>(512).fill(0);
const GF_LOG = new Array<number>(256).fill(0);
const ERROR_GENERATORS = new Map<number, number[]>();

initGaloisField();

export function createQrSvg(text: string, options: QrSvgOptions = {}): string {
  const bytes = Array.from(new TextEncoder().encode(text));
  const spec = selectQrSpec(bytes.length);
  const codewords = addErrorCorrection(encodeData(bytes, spec), spec);
  const base = createBaseMatrix(spec);
  const dataCoords = placeDataBits(base, codewords, spec);

  let bestMatrix: boolean[][] | null = null;
  let bestPenalty = Number.POSITIVE_INFINITY;
  let bestMask = 0;

  for (let mask = 0; mask < 8; mask += 1) {
    const candidate = applyMask(base.modules, dataCoords, mask);
    writeFormatInfo(candidate, mask, spec);
    const candidatePenalty = penaltyScore(candidate);
    if (candidatePenalty < bestPenalty) {
      bestMatrix = candidate;
      bestPenalty = candidatePenalty;
      bestMask = mask;
    }
  }

  if (!bestMatrix) {
    throw new Error("QR matrix generation failed");
  }

  writeFormatInfo(bestMatrix, bestMask, spec);
  return matrixToSvg(bestMatrix, options);
}

function qrSpec(
  version: number,
  alignmentPositions: number[],
  ecCodewordsPerBlock: number,
  blocks: QrBlockGroup[],
): QrVersionSpec {
  const dataCodewords = blocks.reduce((total, block) => total + block.count * block.dataCodewords, 0);
  const totalCodewords = blocks.reduce(
    (total, block) => total + block.count * (block.dataCodewords + ecCodewordsPerBlock),
    0,
  );
  return {
    alignmentPositions,
    blocks,
    dataCodewords,
    ecCodewordsPerBlock,
    lengthBits: version <= 9 ? 8 : 16,
    size: 17 + version * 4,
    totalCodewords,
    version,
  };
}

function selectQrSpec(byteLength: number) {
  const spec = QR_SPECS.find((candidate) => {
    const capacityBits = candidate.dataCodewords * 8;
    return 4 + candidate.lengthBits + byteLength * 8 <= capacityBits;
  });
  if (!spec) {
    throw new Error("QR payload is too long");
  }
  return spec;
}

function encodeData(bytes: number[], spec: QrVersionSpec): number[] {
  const capacityBits = spec.dataCodewords * 8;
  if (bytes.length * 8 + 4 + spec.lengthBits > capacityBits) {
    throw new Error("QR payload is too long");
  }

  const bits: number[] = [];
  pushBits(bits, BYTE_MODE, 4);
  pushBits(bits, bytes.length, spec.lengthBits);
  for (const byte of bytes) {
    pushBits(bits, byte, 8);
  }

  const terminatorLength = Math.min(4, capacityBits - bits.length);
  pushBits(bits, 0, terminatorLength);
  while (bits.length % 8 !== 0) {
    bits.push(0);
  }

  const data = bitsToBytes(bits);
  let padIndex = 0;
  while (data.length < spec.dataCodewords) {
    data.push(PAD_CODEWORDS[padIndex % PAD_CODEWORDS.length]);
    padIndex += 1;
  }
  return data;
}

function pushBits(bits: number[], value: number, length: number) {
  for (let shift = length - 1; shift >= 0; shift -= 1) {
    bits.push((value >>> shift) & 1);
  }
}

function bitsToBytes(bits: number[]): number[] {
  const bytes: number[] = [];
  for (let index = 0; index < bits.length; index += 8) {
    let value = 0;
    for (let offset = 0; offset < 8; offset += 1) {
      value = (value << 1) | (bits[index + offset] || 0);
    }
    bytes.push(value);
  }
  return bytes;
}

function addErrorCorrection(data: number[], spec: QrVersionSpec): number[] {
  const blocks = splitDataBlocks(data, spec);
  const generator = errorGenerator(spec.ecCodewordsPerBlock);
  const errorBlocks = blocks.map((block) => reedSolomonRemainder(block, generator));
  const result: number[] = [];
  const maxDataCodewords = Math.max(...blocks.map((block) => block.length));

  for (let index = 0; index < maxDataCodewords; index += 1) {
    for (const block of blocks) {
      if (index < block.length) {
        result.push(block[index]);
      }
    }
  }
  for (let index = 0; index < spec.ecCodewordsPerBlock; index += 1) {
    for (const block of errorBlocks) {
      result.push(block[index]);
    }
  }
  if (result.length !== spec.totalCodewords) {
    throw new Error("QR error correction failed");
  }
  return result;
}

function splitDataBlocks(data: number[], spec: QrVersionSpec) {
  const blocks: number[][] = [];
  let offset = 0;
  for (const group of spec.blocks) {
    for (let index = 0; index < group.count; index += 1) {
      blocks.push(data.slice(offset, offset + group.dataCodewords));
      offset += group.dataCodewords;
    }
  }
  if (offset !== data.length) {
    throw new Error("QR data block split failed");
  }
  return blocks;
}

function createBaseMatrix(spec: QrVersionSpec): MatrixState {
  const modules = Array.from({ length: spec.size }, () => Array<boolean | null>(spec.size).fill(null));
  const reserved = Array.from({ length: spec.size }, () => Array<boolean>(spec.size).fill(false));
  const state = { modules, reserved };

  drawFinderPattern(state, 0, 0);
  drawFinderPattern(state, spec.size - 7, 0);
  drawFinderPattern(state, 0, spec.size - 7);
  drawAlignmentPatterns(state, spec);
  drawTimingPatterns(state);
  drawVersionInfo(state, spec);
  reserveFormatInfo(state, spec);
  setFunctionModule(state, 8, 4 * spec.version + 9, true);
  return state;
}

function drawFinderPattern(state: MatrixState, startX: number, startY: number) {
  for (let dy = -1; dy <= 7; dy += 1) {
    for (let dx = -1; dx <= 7; dx += 1) {
      const x = startX + dx;
      const y = startY + dy;
      if (!inBounds(state, x, y)) {
        continue;
      }

      const inPattern = dx >= 0 && dx <= 6 && dy >= 0 && dy <= 6;
      const dark =
        inPattern &&
        (dx === 0 ||
          dx === 6 ||
          dy === 0 ||
          dy === 6 ||
          (dx >= 2 && dx <= 4 && dy >= 2 && dy <= 4));
      setFunctionModule(state, x, y, dark);
    }
  }
}

function drawAlignmentPatterns(state: MatrixState, spec: QrVersionSpec) {
  for (const centerY of spec.alignmentPositions) {
    for (const centerX of spec.alignmentPositions) {
      const overlapsFinder =
        (centerX === 6 && centerY === 6) ||
        (centerX === 6 && centerY === spec.size - 7) ||
        (centerX === spec.size - 7 && centerY === 6);
      if (overlapsFinder) {
        continue;
      }

      for (let dy = -2; dy <= 2; dy += 1) {
        for (let dx = -2; dx <= 2; dx += 1) {
          const distance = Math.max(Math.abs(dx), Math.abs(dy));
          setFunctionModule(state, centerX + dx, centerY + dy, distance !== 1);
        }
      }
    }
  }
}

function drawTimingPatterns(state: MatrixState) {
  const size = state.modules.length;
  for (let index = 8; index < size - 8; index += 1) {
    const dark = index % 2 === 0;
    setFunctionModule(state, index, 6, dark);
    setFunctionModule(state, 6, index, dark);
  }
}

function drawVersionInfo(state: MatrixState, spec: QrVersionSpec) {
  const bits = versionBits(spec.version);
  for (let index = 0; index < 18; index += 1) {
    const dark = ((bits >>> index) & 1) === 1;
    const a = spec.size - 11 + (index % 3);
    const b = Math.floor(index / 3);
    setFunctionModule(state, a, b, dark);
    setFunctionModule(state, b, a, dark);
  }
}

function reserveFormatInfo(state: MatrixState, spec: QrVersionSpec) {
  for (let index = 0; index <= 5; index += 1) {
    setFunctionModule(state, 8, index, false);
    setFunctionModule(state, index, 8, false);
  }
  setFunctionModule(state, 8, 7, false);
  setFunctionModule(state, 8, 8, false);
  setFunctionModule(state, 7, 8, false);

  for (let index = 9; index < 15; index += 1) {
    setFunctionModule(state, 14 - index, 8, false);
  }
  for (let index = 0; index < 8; index += 1) {
    setFunctionModule(state, spec.size - 1 - index, 8, false);
  }
  for (let index = 8; index < 15; index += 1) {
    setFunctionModule(state, 8, spec.size - 15 + index, false);
  }
}

function setFunctionModule(state: MatrixState, x: number, y: number, dark: boolean) {
  if (!inBounds(state, x, y)) {
    return;
  }
  state.modules[y][x] = dark;
  state.reserved[y][x] = true;
}

function placeDataBits(state: MatrixState, codewords: number[], spec: QrVersionSpec): DataCoord[] {
  const dataCoords: DataCoord[] = [];
  const totalBits = codewords.length * 8;
  let bitIndex = 0;
  let upward = true;

  for (let right = spec.size - 1; right >= 1; right -= 2) {
    if (right === 6) {
      right -= 1;
    }

    for (let vertical = 0; vertical < spec.size; vertical += 1) {
      const y = upward ? spec.size - 1 - vertical : vertical;
      for (let offset = 0; offset < 2; offset += 1) {
        const x = right - offset;
        if (state.reserved[y][x]) {
          continue;
        }

        const byte = codewords[Math.floor(bitIndex / 8)] || 0;
        const bit = ((byte >>> (7 - (bitIndex % 8))) & 1) === 1;
        state.modules[y][x] = bit;
        dataCoords.push({ x, y });
        bitIndex += 1;
      }
    }
    upward = !upward;
  }

  if (bitIndex < totalBits) {
    throw new Error("QR data placement failed");
  }
  return dataCoords;
}

function applyMask(baseModules: (boolean | null)[][], dataCoords: DataCoord[], mask: number): boolean[][] {
  const matrix = baseModules.map((row) => row.map((value) => value === true));
  for (const coord of dataCoords) {
    if (maskApplies(mask, coord.x, coord.y)) {
      matrix[coord.y][coord.x] = !matrix[coord.y][coord.x];
    }
  }
  return matrix;
}

function maskApplies(mask: number, x: number, y: number): boolean {
  switch (mask) {
    case 0:
      return (x + y) % 2 === 0;
    case 1:
      return y % 2 === 0;
    case 2:
      return x % 3 === 0;
    case 3:
      return (x + y) % 3 === 0;
    case 4:
      return (Math.floor(y / 2) + Math.floor(x / 3)) % 2 === 0;
    case 5:
      return ((x * y) % 2) + ((x * y) % 3) === 0;
    case 6:
      return (((x * y) % 2) + ((x * y) % 3)) % 2 === 0;
    case 7:
      return (((x + y) % 2) + ((x * y) % 3)) % 2 === 0;
    default:
      return false;
  }
}

function writeFormatInfo(matrix: boolean[][], mask: number, spec: QrVersionSpec) {
  const bits = formatBits(mask);
  const bit = (index: number) => ((bits >>> index) & 1) === 1;

  for (let index = 0; index <= 5; index += 1) {
    matrix[index][8] = bit(index);
    matrix[8][index] = bit(index);
  }
  matrix[7][8] = bit(6);
  matrix[8][8] = bit(7);
  matrix[8][7] = bit(8);

  for (let index = 9; index < 15; index += 1) {
    matrix[8][14 - index] = bit(index);
  }
  for (let index = 0; index < 8; index += 1) {
    matrix[8][spec.size - 1 - index] = bit(index);
  }
  for (let index = 8; index < 15; index += 1) {
    matrix[spec.size - 15 + index][8] = bit(index);
  }
  matrix[4 * spec.version + 9][8] = true;
}

function formatBits(mask: number): number {
  const data = (FORMAT_ECL_LOW << 3) | mask;
  let remainder = data << 10;
  const generator = 0x537;
  for (let bit = 14; bit >= 10; bit -= 1) {
    if (((remainder >>> bit) & 1) !== 0) {
      remainder ^= generator << (bit - 10);
    }
  }
  return ((data << 10) | remainder) ^ 0x5412;
}

function versionBits(version: number): number {
  let remainder = version << 12;
  const generator = 0x1f25;
  for (let bit = 17; bit >= 12; bit -= 1) {
    if (((remainder >>> bit) & 1) !== 0) {
      remainder ^= generator << (bit - 12);
    }
  }
  return (version << 12) | remainder;
}

function penaltyScore(matrix: boolean[][]): number {
  return (
    runPenalty(matrix) +
    blockPenalty(matrix) +
    finderPenalty(matrix) +
    balancePenalty(matrix)
  );
}

function runPenalty(matrix: boolean[][]): number {
  let penalty = 0;
  for (let y = 0; y < matrix.length; y += 1) {
    penalty += lineRunPenalty(matrix[y]);
  }
  for (let x = 0; x < matrix.length; x += 1) {
    const column = matrix.map((row) => row[x]);
    penalty += lineRunPenalty(column);
  }
  return penalty;
}

function lineRunPenalty(line: boolean[]): number {
  let penalty = 0;
  let runColor = line[0];
  let runLength = 1;
  for (let index = 1; index < line.length; index += 1) {
    if (line[index] === runColor) {
      runLength += 1;
      continue;
    }
    if (runLength >= 5) {
      penalty += runLength - 2;
    }
    runColor = line[index];
    runLength = 1;
  }
  if (runLength >= 5) {
    penalty += runLength - 2;
  }
  return penalty;
}

function blockPenalty(matrix: boolean[][]): number {
  let penalty = 0;
  for (let y = 0; y < matrix.length - 1; y += 1) {
    for (let x = 0; x < matrix.length - 1; x += 1) {
      const color = matrix[y][x];
      if (matrix[y][x + 1] === color && matrix[y + 1][x] === color && matrix[y + 1][x + 1] === color) {
        penalty += 3;
      }
    }
  }
  return penalty;
}

function finderPenalty(matrix: boolean[][]): number {
  const pattern = [true, false, true, true, true, false, true, false, false, false, false];
  const reverse = [false, false, false, false, true, false, true, true, true, false, true];
  let penalty = 0;

  for (let y = 0; y < matrix.length; y += 1) {
    penalty += patternPenalty(matrix[y], pattern, reverse);
  }
  for (let x = 0; x < matrix.length; x += 1) {
    const column = matrix.map((row) => row[x]);
    penalty += patternPenalty(column, pattern, reverse);
  }
  return penalty;
}

function patternPenalty(line: boolean[], pattern: boolean[], reverse: boolean[]): number {
  let penalty = 0;
  for (let index = 0; index <= line.length - pattern.length; index += 1) {
    if (matchesPattern(line, index, pattern) || matchesPattern(line, index, reverse)) {
      penalty += 40;
    }
  }
  return penalty;
}

function matchesPattern(line: boolean[], start: number, pattern: boolean[]): boolean {
  for (let offset = 0; offset < pattern.length; offset += 1) {
    if (line[start + offset] !== pattern[offset]) {
      return false;
    }
  }
  return true;
}

function balancePenalty(matrix: boolean[][]): number {
  let dark = 0;
  for (const row of matrix) {
    for (const module of row) {
      if (module) {
        dark += 1;
      }
    }
  }
  const total = matrix.length * matrix.length;
  return Math.floor(Math.abs(dark * 20 - total * 10) / total) * 10;
}

function matrixToSvg(matrix: boolean[][], options: QrSvgOptions): string {
  const quietZone = options.quietZone ?? 4;
  const moduleSize = options.moduleSize ?? 6;
  const size = matrix.length;
  const unitSize = size + quietZone * 2;
  const requestedPixelSize = unitSize * moduleSize;
  const maxPixelSize = options.maxPixelSize;
  const pixelSize =
    typeof maxPixelSize === "number" && Number.isFinite(maxPixelSize) && maxPixelSize > 0
      ? Math.min(requestedPixelSize, maxPixelSize)
      : requestedPixelSize;
  const path: string[] = [];

  for (let y = 0; y < size; y += 1) {
    for (let x = 0; x < size; x += 1) {
      if (matrix[y][x]) {
        path.push(`M${x + quietZone},${y + quietZone}h1v1h-1z`);
      }
    }
  }

  return `<svg xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Connection QR code" viewBox="0 0 ${unitSize} ${unitSize}" width="${pixelSize}" height="${pixelSize}"><rect width="100%" height="100%" fill="#fff"/><path d="${path.join("")}" fill="#101114"/></svg>`;
}

function errorGenerator(degree: number) {
  const cached = ERROR_GENERATORS.get(degree);
  if (cached) {
    return cached;
  }
  const generator = reedSolomonGenerator(degree);
  ERROR_GENERATORS.set(degree, generator);
  return generator;
}

function reedSolomonGenerator(degree: number): number[] {
  let result = [1];
  for (let index = 0; index < degree; index += 1) {
    result = polynomialMultiply(result, [1, GF_EXP[index]]);
  }
  return result;
}

function reedSolomonRemainder(data: number[], generator: number[]): number[] {
  const degree = generator.length - 1;
  const result = new Array<number>(degree).fill(0);
  for (const byte of data) {
    const factor = byte ^ result.shift()!;
    result.push(0);
    for (let index = 0; index < degree; index += 1) {
      result[index] ^= gfMultiply(generator[index + 1], factor);
    }
  }
  return result;
}

function polynomialMultiply(left: number[], right: number[]): number[] {
  const result = new Array<number>(left.length + right.length - 1).fill(0);
  for (let i = 0; i < left.length; i += 1) {
    for (let j = 0; j < right.length; j += 1) {
      result[i + j] ^= gfMultiply(left[i], right[j]);
    }
  }
  return result;
}

function initGaloisField() {
  let value = 1;
  for (let index = 0; index < 255; index += 1) {
    GF_EXP[index] = value;
    GF_LOG[value] = index;
    value <<= 1;
    if ((value & 0x100) !== 0) {
      value ^= 0x11d;
    }
  }
  for (let index = 255; index < GF_EXP.length; index += 1) {
    GF_EXP[index] = GF_EXP[index - 255];
  }
}

function gfMultiply(left: number, right: number): number {
  if (left === 0 || right === 0) {
    return 0;
  }
  return GF_EXP[GF_LOG[left] + GF_LOG[right]];
}

function inBounds(state: MatrixState, x: number, y: number): boolean {
  return x >= 0 && x < state.modules.length && y >= 0 && y < state.modules.length;
}
