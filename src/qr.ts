type QrSvgOptions = {
  moduleSize?: number;
  quietZone?: number;
};

type DataCoord = {
  x: number;
  y: number;
};

type MatrixState = {
  modules: (boolean | null)[][];
  reserved: boolean[][];
};

const QR_VERSION = 7;
const QR_SIZE = 45;
const DATA_CODEWORDS = 156;
const DATA_CODEWORDS_PER_BLOCK = 78;
const ERROR_CODEWORDS_PER_BLOCK = 20;
const BYTE_MODE = 0b0100;
const FORMAT_ECL_LOW = 0b01;
const PAD_CODEWORDS = [0xec, 0x11];
const ALIGNMENT_POSITIONS = [6, 22, 38];
const GF_EXP = new Array<number>(512).fill(0);
const GF_LOG = new Array<number>(256).fill(0);

initGaloisField();

const ERROR_GENERATOR = reedSolomonGenerator(ERROR_CODEWORDS_PER_BLOCK);

export function createQrSvg(text: string, options: QrSvgOptions = {}): string {
  const codewords = addErrorCorrection(encodeData(text));
  const base = createBaseMatrix();
  const dataCoords = placeDataBits(base, codewords);

  let bestMatrix: boolean[][] | null = null;
  let bestPenalty = Number.POSITIVE_INFINITY;
  let bestMask = 0;

  for (let mask = 0; mask < 8; mask += 1) {
    const candidate = applyMask(base.modules, dataCoords, mask);
    writeFormatInfo(candidate, mask);
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

  writeFormatInfo(bestMatrix, bestMask);
  return matrixToSvg(bestMatrix, options);
}

function encodeData(text: string): number[] {
  const bytes = Array.from(new TextEncoder().encode(text));
  const capacityBits = DATA_CODEWORDS * 8;
  if (bytes.length * 8 + 12 > capacityBits) {
    throw new Error("QR payload is too long");
  }

  const bits: number[] = [];
  pushBits(bits, BYTE_MODE, 4);
  pushBits(bits, bytes.length, 8);
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
  while (data.length < DATA_CODEWORDS) {
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

function addErrorCorrection(data: number[]): number[] {
  const firstBlock = data.slice(0, DATA_CODEWORDS_PER_BLOCK);
  const secondBlock = data.slice(DATA_CODEWORDS_PER_BLOCK);
  const blocks = [firstBlock, secondBlock];
  const errorBlocks = blocks.map((block) => reedSolomonRemainder(block, ERROR_GENERATOR));
  const result: number[] = [];

  for (let index = 0; index < DATA_CODEWORDS_PER_BLOCK; index += 1) {
    for (const block of blocks) {
      result.push(block[index]);
    }
  }
  for (let index = 0; index < ERROR_CODEWORDS_PER_BLOCK; index += 1) {
    for (const block of errorBlocks) {
      result.push(block[index]);
    }
  }
  return result;
}

function createBaseMatrix(): MatrixState {
  const modules = Array.from({ length: QR_SIZE }, () => Array<boolean | null>(QR_SIZE).fill(null));
  const reserved = Array.from({ length: QR_SIZE }, () => Array<boolean>(QR_SIZE).fill(false));
  const state = { modules, reserved };

  drawFinderPattern(state, 0, 0);
  drawFinderPattern(state, QR_SIZE - 7, 0);
  drawFinderPattern(state, 0, QR_SIZE - 7);
  drawAlignmentPatterns(state);
  drawTimingPatterns(state);
  drawVersionInfo(state);
  reserveFormatInfo(state);
  setFunctionModule(state, 8, 4 * QR_VERSION + 9, true);
  return state;
}

function drawFinderPattern(state: MatrixState, startX: number, startY: number) {
  for (let dy = -1; dy <= 7; dy += 1) {
    for (let dx = -1; dx <= 7; dx += 1) {
      const x = startX + dx;
      const y = startY + dy;
      if (!inBounds(x, y)) {
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

function drawAlignmentPatterns(state: MatrixState) {
  for (const centerY of ALIGNMENT_POSITIONS) {
    for (const centerX of ALIGNMENT_POSITIONS) {
      const overlapsFinder =
        (centerX === 6 && centerY === 6) ||
        (centerX === 6 && centerY === QR_SIZE - 7) ||
        (centerX === QR_SIZE - 7 && centerY === 6);
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
  for (let index = 8; index < QR_SIZE - 8; index += 1) {
    const dark = index % 2 === 0;
    setFunctionModule(state, index, 6, dark);
    setFunctionModule(state, 6, index, dark);
  }
}

function drawVersionInfo(state: MatrixState) {
  const bits = versionBits();
  for (let index = 0; index < 18; index += 1) {
    const dark = ((bits >>> index) & 1) === 1;
    const a = QR_SIZE - 11 + (index % 3);
    const b = Math.floor(index / 3);
    setFunctionModule(state, a, b, dark);
    setFunctionModule(state, b, a, dark);
  }
}

function reserveFormatInfo(state: MatrixState) {
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
    setFunctionModule(state, QR_SIZE - 1 - index, 8, false);
  }
  for (let index = 8; index < 15; index += 1) {
    setFunctionModule(state, 8, QR_SIZE - 15 + index, false);
  }
}

function setFunctionModule(state: MatrixState, x: number, y: number, dark: boolean) {
  if (!inBounds(x, y)) {
    return;
  }
  state.modules[y][x] = dark;
  state.reserved[y][x] = true;
}

function placeDataBits(state: MatrixState, codewords: number[]): DataCoord[] {
  const dataCoords: DataCoord[] = [];
  const totalBits = codewords.length * 8;
  let bitIndex = 0;
  let upward = true;

  for (let right = QR_SIZE - 1; right >= 1; right -= 2) {
    if (right === 6) {
      right -= 1;
    }

    for (let vertical = 0; vertical < QR_SIZE; vertical += 1) {
      const y = upward ? QR_SIZE - 1 - vertical : vertical;
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

  if (bitIndex !== totalBits) {
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

function writeFormatInfo(matrix: boolean[][], mask: number) {
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
    matrix[8][QR_SIZE - 1 - index] = bit(index);
  }
  for (let index = 8; index < 15; index += 1) {
    matrix[QR_SIZE - 15 + index][8] = bit(index);
  }
  matrix[4 * QR_VERSION + 9][8] = true;
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

function versionBits(): number {
  let remainder = QR_VERSION << 12;
  const generator = 0x1f25;
  for (let bit = 17; bit >= 12; bit -= 1) {
    if (((remainder >>> bit) & 1) !== 0) {
      remainder ^= generator << (bit - 12);
    }
  }
  return (QR_VERSION << 12) | remainder;
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
  for (let y = 0; y < QR_SIZE; y += 1) {
    penalty += lineRunPenalty(matrix[y]);
  }
  for (let x = 0; x < QR_SIZE; x += 1) {
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
  for (let y = 0; y < QR_SIZE - 1; y += 1) {
    for (let x = 0; x < QR_SIZE - 1; x += 1) {
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

  for (let y = 0; y < QR_SIZE; y += 1) {
    penalty += patternPenalty(matrix[y], pattern, reverse);
  }
  for (let x = 0; x < QR_SIZE; x += 1) {
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
  const total = QR_SIZE * QR_SIZE;
  return Math.floor(Math.abs(dark * 20 - total * 10) / total) * 10;
}

function matrixToSvg(matrix: boolean[][], options: QrSvgOptions): string {
  const quietZone = options.quietZone ?? 4;
  const moduleSize = options.moduleSize ?? 6;
  const unitSize = QR_SIZE + quietZone * 2;
  const pixelSize = unitSize * moduleSize;
  const path: string[] = [];

  for (let y = 0; y < QR_SIZE; y += 1) {
    for (let x = 0; x < QR_SIZE; x += 1) {
      if (matrix[y][x]) {
        path.push(`M${x + quietZone},${y + quietZone}h1v1h-1z`);
      }
    }
  }

  return `<svg xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Connection QR code" viewBox="0 0 ${unitSize} ${unitSize}" width="${pixelSize}" height="${pixelSize}"><rect width="100%" height="100%" fill="#fff"/><path d="${path.join("")}" fill="#101114"/></svg>`;
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

function inBounds(x: number, y: number): boolean {
  return x >= 0 && x < QR_SIZE && y >= 0 && y < QR_SIZE;
}
