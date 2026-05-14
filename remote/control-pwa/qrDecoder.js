const QR_VERSION = 7;
const QR_SIZE = 45;
const DATA_CODEWORDS = 156;
const DATA_CODEWORDS_PER_BLOCK = 78;
const TOTAL_CODEWORDS = 196;
const BYTE_MODE = 0b0100;
const ALIGNMENT_POSITIONS = [6, 22, 38];
const SCAN_MAX_SIDE = 720;

const scanCanvas = document.createElement("canvas");
const scanContext = scanCanvas.getContext("2d", { willReadFrequently: true });
const dataCoords = buildDataCoords();
const expectedFunctions = buildExpectedFunctions();
const textDecoder = new TextDecoder("utf-8", { fatal: false });

export function decodeCodexQrFromVideo(video) {
  if (!scanContext || !video.videoWidth || !video.videoHeight) {
    return null;
  }

  const dimensions = scaledDimensions(video.videoWidth, video.videoHeight);
  if (scanCanvas.width !== dimensions.width || scanCanvas.height !== dimensions.height) {
    scanCanvas.width = dimensions.width;
    scanCanvas.height = dimensions.height;
  }

  scanContext.drawImage(video, 0, 0, dimensions.width, dimensions.height);
  const image = scanContext.getImageData(0, 0, dimensions.width, dimensions.height);
  return decodeCodexQrFromImageData(image);
}

export function decodeCodexQrFromImageData(image) {
  const libraryPayload = decodeWithJsQr(image);
  if (libraryPayload) {
    return libraryPayload;
  }

  const luminance = extractLuminance(image.data);
  const threshold = otsuThreshold(luminance);
  const binary = binarize(luminance, threshold);
  const finders = findFinderPatterns(binary, image.width, image.height);
  return decodeFromFinderPatterns(binary, image.width, image.height, finders);
}

function decodeWithJsQr(image) {
  const decoder = typeof globalThis.jsQR === "function" ? globalThis.jsQR : null;
  if (!decoder) {
    return null;
  }

  try {
    const result = decoder(image.data, image.width, image.height, {
      inversionAttempts: "attemptBoth",
    });
    return typeof result?.data === "string" && result.data.trim() ? result.data : null;
  } catch {
    return null;
  }
}

function scaledDimensions(width, height) {
  const maxSide = Math.max(width, height);
  const scale = maxSide > SCAN_MAX_SIDE ? SCAN_MAX_SIDE / maxSide : 1;
  return {
    height: Math.max(1, Math.round(height * scale)),
    width: Math.max(1, Math.round(width * scale)),
  };
}

function extractLuminance(data) {
  const luminance = new Uint8Array(data.length / 4);
  for (let source = 0, target = 0; source < data.length; source += 4, target += 1) {
    luminance[target] = (data[source] * 299 + data[source + 1] * 587 + data[source + 2] * 114) / 1000;
  }
  return luminance;
}

function otsuThreshold(luminance) {
  const histogram = new Uint32Array(256);
  for (const value of luminance) {
    histogram[value] += 1;
  }

  let total = luminance.length;
  let sum = 0;
  for (let value = 0; value < histogram.length; value += 1) {
    sum += value * histogram[value];
  }

  let backgroundWeight = 0;
  let backgroundSum = 0;
  let bestVariance = -1;
  let bestThreshold = 128;

  for (let threshold = 0; threshold < histogram.length; threshold += 1) {
    backgroundWeight += histogram[threshold];
    if (backgroundWeight === 0) {
      continue;
    }

    const foregroundWeight = total - backgroundWeight;
    if (foregroundWeight === 0) {
      break;
    }

    backgroundSum += threshold * histogram[threshold];
    const backgroundMean = backgroundSum / backgroundWeight;
    const foregroundMean = (sum - backgroundSum) / foregroundWeight;
    const variance =
      backgroundWeight * foregroundWeight * (backgroundMean - foregroundMean) * (backgroundMean - foregroundMean);

    if (variance > bestVariance) {
      bestVariance = variance;
      bestThreshold = threshold;
    }
  }

  return bestThreshold;
}

function binarize(luminance, threshold) {
  const binary = new Uint8Array(luminance.length);
  for (let index = 0; index < luminance.length; index += 1) {
    binary[index] = luminance[index] <= threshold ? 1 : 0;
  }
  return binary;
}

function findFinderPatterns(binary, width, height) {
  const candidates = [];

  for (let y = 0; y < height; y += 1) {
    const counts = [0, 0, 0, 0, 0];
    let state = 0;

    for (let x = 0; x < width; x += 1) {
      if (isDark(binary, width, x, y)) {
        if ((state & 1) === 1) {
          state += 1;
        }
        counts[state] += 1;
        continue;
      }

      if ((state & 1) === 0) {
        if (state === 4) {
          if (finderCountsMatch(counts) && handlePossibleCenter(binary, width, height, counts, y, x, candidates)) {
            state = 0;
            counts.fill(0);
          } else {
            shiftCounts(counts);
            state = 3;
          }
        } else {
          state += 1;
          counts[state] += 1;
        }
      } else {
        counts[state] += 1;
      }
    }

    if (finderCountsMatch(counts)) {
      handlePossibleCenter(binary, width, height, counts, y, width, candidates);
    }
  }

  return candidates
    .filter((candidate) => candidate.count >= 2)
    .sort((left, right) => right.count - left.count)
    .slice(0, 24);
}

function shiftCounts(counts) {
  counts[0] = counts[2];
  counts[1] = counts[3];
  counts[2] = counts[4];
  counts[3] = 1;
  counts[4] = 0;
}

function handlePossibleCenter(binary, width, height, counts, row, endX, candidates) {
  const total = sum(counts);
  const centerX = centerFromEnd(counts, endX);
  const maxCount = Math.max(...counts);
  const centerY = crossCheckVertical(binary, width, height, Math.round(centerX), row, maxCount, total);
  if (!Number.isFinite(centerY)) {
    return false;
  }

  const checkedCenterX = crossCheckHorizontal(binary, width, Math.round(centerX), Math.round(centerY), maxCount, total);
  if (!Number.isFinite(checkedCenterX)) {
    return false;
  }

  addFinderCandidate(candidates, checkedCenterX, centerY, total / 7);
  return true;
}

function crossCheckVertical(binary, width, height, startX, startY, maxCount, originalTotal) {
  const counts = [0, 0, 0, 0, 0];
  let y = startY;

  while (y >= 0 && isDark(binary, width, startX, y)) {
    counts[2] += 1;
    y -= 1;
  }
  if (y < 0) {
    return Number.NaN;
  }
  while (y >= 0 && !isDark(binary, width, startX, y) && counts[1] <= maxCount) {
    counts[1] += 1;
    y -= 1;
  }
  if (y < 0 || counts[1] > maxCount) {
    return Number.NaN;
  }
  while (y >= 0 && isDark(binary, width, startX, y) && counts[0] <= maxCount) {
    counts[0] += 1;
    y -= 1;
  }
  if (counts[0] > maxCount) {
    return Number.NaN;
  }

  y = startY + 1;
  while (y < height && isDark(binary, width, startX, y)) {
    counts[2] += 1;
    y += 1;
  }
  if (y === height) {
    return Number.NaN;
  }
  while (y < height && !isDark(binary, width, startX, y) && counts[3] < maxCount) {
    counts[3] += 1;
    y += 1;
  }
  if (y === height || counts[3] >= maxCount) {
    return Number.NaN;
  }
  while (y < height && isDark(binary, width, startX, y) && counts[4] < maxCount) {
    counts[4] += 1;
    y += 1;
  }
  if (counts[4] >= maxCount) {
    return Number.NaN;
  }

  const total = sum(counts);
  if (Math.abs(total - originalTotal) > originalTotal) {
    return Number.NaN;
  }

  return finderCountsMatch(counts) ? centerFromEnd(counts, y) : Number.NaN;
}

function crossCheckHorizontal(binary, width, startX, startY, maxCount, originalTotal) {
  const counts = [0, 0, 0, 0, 0];
  let x = startX;

  while (x >= 0 && isDark(binary, width, x, startY)) {
    counts[2] += 1;
    x -= 1;
  }
  if (x < 0) {
    return Number.NaN;
  }
  while (x >= 0 && !isDark(binary, width, x, startY) && counts[1] <= maxCount) {
    counts[1] += 1;
    x -= 1;
  }
  if (x < 0 || counts[1] > maxCount) {
    return Number.NaN;
  }
  while (x >= 0 && isDark(binary, width, x, startY) && counts[0] <= maxCount) {
    counts[0] += 1;
    x -= 1;
  }
  if (counts[0] > maxCount) {
    return Number.NaN;
  }

  x = startX + 1;
  while (x < width && isDark(binary, width, x, startY)) {
    counts[2] += 1;
    x += 1;
  }
  if (x === width) {
    return Number.NaN;
  }
  while (x < width && !isDark(binary, width, x, startY) && counts[3] < maxCount) {
    counts[3] += 1;
    x += 1;
  }
  if (x === width || counts[3] >= maxCount) {
    return Number.NaN;
  }
  while (x < width && isDark(binary, width, x, startY) && counts[4] < maxCount) {
    counts[4] += 1;
    x += 1;
  }
  if (counts[4] >= maxCount) {
    return Number.NaN;
  }

  const total = sum(counts);
  if (Math.abs(total - originalTotal) > originalTotal) {
    return Number.NaN;
  }

  return finderCountsMatch(counts) ? centerFromEnd(counts, x) : Number.NaN;
}

function finderCountsMatch(counts) {
  if (counts.some((count) => count === 0)) {
    return false;
  }

  const total = sum(counts);
  if (total < 7) {
    return false;
  }

  const moduleSize = total / 7;
  const maxVariance = moduleSize * 0.82;
  return (
    Math.abs(moduleSize - counts[0]) < maxVariance &&
    Math.abs(moduleSize - counts[1]) < maxVariance &&
    Math.abs(moduleSize * 3 - counts[2]) < maxVariance * 3 &&
    Math.abs(moduleSize - counts[3]) < maxVariance &&
    Math.abs(moduleSize - counts[4]) < maxVariance
  );
}

function centerFromEnd(counts, end) {
  return end - counts[4] - counts[3] - counts[2] / 2;
}

function addFinderCandidate(candidates, x, y, moduleSize) {
  for (const candidate of candidates) {
    const tolerance = Math.max(moduleSize, candidate.moduleSize) * 2.2;
    if (
      Math.abs(candidate.x - x) <= tolerance &&
      Math.abs(candidate.y - y) <= tolerance &&
      Math.abs(candidate.moduleSize - moduleSize) <= Math.max(moduleSize, candidate.moduleSize)
    ) {
      const nextCount = candidate.count + 1;
      candidate.x = (candidate.x * candidate.count + x) / nextCount;
      candidate.y = (candidate.y * candidate.count + y) / nextCount;
      candidate.moduleSize = (candidate.moduleSize * candidate.count + moduleSize) / nextCount;
      candidate.count = nextCount;
      return;
    }
  }

  candidates.push({ count: 1, moduleSize, x, y });
}

function decodeFromFinderPatterns(binary, width, height, finders) {
  for (let a = 0; a < finders.length - 2; a += 1) {
    for (let b = a + 1; b < finders.length - 1; b += 1) {
      for (let c = b + 1; c < finders.length; c += 1) {
        const ordered = orderFinders(finders[a], finders[b], finders[c]);
        if (!ordered) {
          continue;
        }

        const matrix = sampleQrMatrix(binary, width, height, ordered);
        if (!matrix || functionPatternScore(matrix) < 0.84) {
          continue;
        }

        const payload = decodeFixedQrMatrix(matrix);
        if (payload) {
          return payload;
        }
      }
    }
  }

  return null;
}

function orderFinders(first, second, third) {
  const ab = distanceSquared(first, second);
  const ac = distanceSquared(first, third);
  const bc = distanceSquared(second, third);
  let topLeft;
  let one;
  let two;
  let sideA;
  let sideB;
  let diagonal;

  if (bc >= ab && bc >= ac) {
    topLeft = first;
    one = second;
    two = third;
    sideA = ab;
    sideB = ac;
    diagonal = bc;
  } else if (ac >= ab && ac >= bc) {
    topLeft = second;
    one = first;
    two = third;
    sideA = ab;
    sideB = bc;
    diagonal = ac;
  } else {
    topLeft = third;
    one = first;
    two = second;
    sideA = ac;
    sideB = bc;
    diagonal = ab;
  }

  const sideRatio = Math.max(sideA, sideB) / Math.max(1, Math.min(sideA, sideB));
  const diagonalError = Math.abs(diagonal - sideA - sideB) / Math.max(1, diagonal);
  if (sideRatio > 1.55 || diagonalError > 0.38) {
    return null;
  }

  const sideLength = Math.sqrt((sideA + sideB) / 2);
  const moduleSize = sideLength / 38;
  const averageFinderModuleSize = (first.moduleSize + second.moduleSize + third.moduleSize) / 3;
  const moduleRatio = Math.max(moduleSize, averageFinderModuleSize) / Math.max(1, Math.min(moduleSize, averageFinderModuleSize));
  if (moduleRatio > 2.25 || moduleSize < 1.4) {
    return null;
  }

  const cross = crossProduct(topLeft, one, two);
  if (Math.abs(cross) < sideLength * sideLength * 0.34) {
    return null;
  }

  return cross > 0
    ? { bottomLeft: two, moduleSize, topLeft, topRight: one }
    : { bottomLeft: one, moduleSize, topLeft, topRight: two };
}

function sampleQrMatrix(binary, width, height, ordered) {
  const matrix = Array.from({ length: QR_SIZE }, () => Array(QR_SIZE).fill(false));
  const xAxis = {
    x: (ordered.topRight.x - ordered.topLeft.x) / 38,
    y: (ordered.topRight.y - ordered.topLeft.y) / 38,
  };
  const yAxis = {
    x: (ordered.bottomLeft.x - ordered.topLeft.x) / 38,
    y: (ordered.bottomLeft.y - ordered.topLeft.y) / 38,
  };

  for (let y = 0; y < QR_SIZE; y += 1) {
    for (let x = 0; x < QR_SIZE; x += 1) {
      const moduleX = x + 0.5 - 3.5;
      const moduleY = y + 0.5 - 3.5;
      const imageX = ordered.topLeft.x + xAxis.x * moduleX + yAxis.x * moduleY;
      const imageY = ordered.topLeft.y + xAxis.y * moduleX + yAxis.y * moduleY;
      if (imageX < 0 || imageY < 0 || imageX >= width || imageY >= height) {
        return null;
      }
      matrix[y][x] = sampleDark(binary, width, height, imageX, imageY, ordered.moduleSize);
    }
  }

  return matrix;
}

function sampleDark(binary, width, height, x, y, moduleSize) {
  const radius = Math.max(1, Math.min(3, Math.floor(moduleSize * 0.22)));
  let dark = 0;
  let total = 0;

  for (const dy of [-radius, 0, radius]) {
    for (const dx of [-radius, 0, radius]) {
      const sampleX = Math.round(x + dx);
      const sampleY = Math.round(y + dy);
      if (sampleX < 0 || sampleY < 0 || sampleX >= width || sampleY >= height) {
        continue;
      }
      total += 1;
      dark += isDark(binary, width, sampleX, sampleY) ? 1 : 0;
    }
  }

  return dark * 2 >= total;
}

function functionPatternScore(matrix) {
  let matches = 0;
  let total = 0;

  for (let y = 0; y < QR_SIZE; y += 1) {
    for (let x = 0; x < QR_SIZE; x += 1) {
      const expected = expectedFunctions[y][x];
      if (expected === null) {
        continue;
      }
      total += 1;
      if (matrix[y][x] === expected) {
        matches += 1;
      }
    }
  }

  return total ? matches / total : 0;
}

function decodeFixedQrMatrix(matrix) {
  for (let mask = 0; mask < 8; mask += 1) {
    const codewords = readCodewords(matrix, mask);
    const payload = decodePayload(deinterleaveDataCodewords(codewords));
    if (payload && isLikelyConnectionPayload(payload)) {
      return payload;
    }
  }
  return null;
}

function readCodewords(matrix, mask) {
  const bits = [];
  for (const coord of dataCoords) {
    let bit = matrix[coord.y][coord.x];
    if (maskApplies(mask, coord.x, coord.y)) {
      bit = !bit;
    }
    bits.push(bit ? 1 : 0);
  }

  const codewords = [];
  for (let index = 0; index < TOTAL_CODEWORDS; index += 1) {
    let value = 0;
    for (let offset = 0; offset < 8; offset += 1) {
      value = (value << 1) | (bits[index * 8 + offset] || 0);
    }
    codewords.push(value);
  }
  return codewords;
}

function deinterleaveDataCodewords(codewords) {
  const firstBlock = [];
  const secondBlock = [];
  for (let index = 0; index < DATA_CODEWORDS_PER_BLOCK; index += 1) {
    firstBlock.push(codewords[index * 2]);
    secondBlock.push(codewords[index * 2 + 1]);
  }
  return firstBlock.concat(secondBlock);
}

function decodePayload(data) {
  const bits = bytesToBits(data);
  let offset = 0;
  const readBits = (length) => {
    if (offset + length > bits.length) {
      return null;
    }
    let value = 0;
    for (let index = 0; index < length; index += 1) {
      value = (value << 1) | bits[offset + index];
    }
    offset += length;
    return value;
  };

  const mode = readBits(4);
  if (mode !== BYTE_MODE) {
    return null;
  }

  const byteLength = readBits(8);
  if (byteLength === null || byteLength <= 0 || byteLength > data.length - 2) {
    return null;
  }

  const payload = [];
  for (let index = 0; index < byteLength; index += 1) {
    const byte = readBits(8);
    if (byte === null) {
      return null;
    }
    payload.push(byte);
  }

  return textDecoder.decode(new Uint8Array(payload));
}

function bytesToBits(bytes) {
  const bits = [];
  for (const byte of bytes) {
    for (let shift = 7; shift >= 0; shift -= 1) {
      bits.push((byte >>> shift) & 1);
    }
  }
  return bits;
}

function isLikelyConnectionPayload(value) {
  const text = value.trim();
  return (
    text.startsWith("http://") ||
    text.startsWith("https://") ||
    (text.startsWith("{") && (text.includes("\"url\"") || text.includes("\"host\"") || text.includes("\"token\"")))
  );
}

function buildDataCoords() {
  const reserved = buildReservedMatrix();
  const coords = [];

  let upward = true;
  for (let right = QR_SIZE - 1; right >= 1; right -= 2) {
    if (right === 6) {
      right -= 1;
    }

    for (let vertical = 0; vertical < QR_SIZE; vertical += 1) {
      const y = upward ? QR_SIZE - 1 - vertical : vertical;
      for (let offset = 0; offset < 2; offset += 1) {
        const x = right - offset;
        if (!reserved[y][x]) {
          coords.push({ x, y });
        }
      }
    }
    upward = !upward;
  }

  return coords;
}

function buildExpectedFunctions() {
  const matrix = Array.from({ length: QR_SIZE }, () => Array(QR_SIZE).fill(null));
  drawFinder(matrix, 0, 0);
  drawFinder(matrix, QR_SIZE - 7, 0);
  drawFinder(matrix, 0, QR_SIZE - 7);
  drawAlignmentPatterns(matrix);
  drawTimingPatterns(matrix);
  drawVersionInfo(matrix);
  matrix[4 * QR_VERSION + 9][8] = true;
  return matrix;
}

function buildReservedMatrix() {
  const matrix = Array.from({ length: QR_SIZE }, () => Array(QR_SIZE).fill(false));
  drawFinderReserved(matrix, 0, 0);
  drawFinderReserved(matrix, QR_SIZE - 7, 0);
  drawFinderReserved(matrix, 0, QR_SIZE - 7);
  drawAlignmentReserved(matrix);
  drawTimingReserved(matrix);
  drawVersionReserved(matrix);
  reserveFormatInfo(matrix);
  matrix[4 * QR_VERSION + 9][8] = true;
  return matrix;
}

function drawFinder(matrix, startX, startY) {
  for (let dy = -1; dy <= 7; dy += 1) {
    for (let dx = -1; dx <= 7; dx += 1) {
      const x = startX + dx;
      const y = startY + dy;
      if (!inQrBounds(x, y)) {
        continue;
      }

      const inPattern = dx >= 0 && dx <= 6 && dy >= 0 && dy <= 6;
      matrix[y][x] =
        inPattern &&
        (dx === 0 || dx === 6 || dy === 0 || dy === 6 || (dx >= 2 && dx <= 4 && dy >= 2 && dy <= 4));
    }
  }
}

function drawFinderReserved(matrix, startX, startY) {
  for (let dy = -1; dy <= 7; dy += 1) {
    for (let dx = -1; dx <= 7; dx += 1) {
      const x = startX + dx;
      const y = startY + dy;
      if (inQrBounds(x, y)) {
        matrix[y][x] = true;
      }
    }
  }
}

function drawAlignmentPatterns(matrix) {
  for (const centerY of ALIGNMENT_POSITIONS) {
    for (const centerX of ALIGNMENT_POSITIONS) {
      if (alignmentOverlapsFinder(centerX, centerY)) {
        continue;
      }

      for (let dy = -2; dy <= 2; dy += 1) {
        for (let dx = -2; dx <= 2; dx += 1) {
          const distance = Math.max(Math.abs(dx), Math.abs(dy));
          matrix[centerY + dy][centerX + dx] = distance !== 1;
        }
      }
    }
  }
}

function drawAlignmentReserved(matrix) {
  for (const centerY of ALIGNMENT_POSITIONS) {
    for (const centerX of ALIGNMENT_POSITIONS) {
      if (alignmentOverlapsFinder(centerX, centerY)) {
        continue;
      }

      for (let dy = -2; dy <= 2; dy += 1) {
        for (let dx = -2; dx <= 2; dx += 1) {
          matrix[centerY + dy][centerX + dx] = true;
        }
      }
    }
  }
}

function alignmentOverlapsFinder(centerX, centerY) {
  return (
    (centerX === 6 && centerY === 6) ||
    (centerX === 6 && centerY === QR_SIZE - 7) ||
    (centerX === QR_SIZE - 7 && centerY === 6)
  );
}

function drawTimingPatterns(matrix) {
  for (let index = 8; index < QR_SIZE - 8; index += 1) {
    const dark = index % 2 === 0;
    matrix[6][index] = dark;
    matrix[index][6] = dark;
  }
}

function drawTimingReserved(matrix) {
  for (let index = 8; index < QR_SIZE - 8; index += 1) {
    matrix[6][index] = true;
    matrix[index][6] = true;
  }
}

function drawVersionInfo(matrix) {
  const bits = versionBits();
  for (let index = 0; index < 18; index += 1) {
    const dark = ((bits >>> index) & 1) === 1;
    const a = QR_SIZE - 11 + (index % 3);
    const b = Math.floor(index / 3);
    matrix[b][a] = dark;
    matrix[a][b] = dark;
  }
}

function drawVersionReserved(matrix) {
  for (let index = 0; index < 18; index += 1) {
    const a = QR_SIZE - 11 + (index % 3);
    const b = Math.floor(index / 3);
    matrix[b][a] = true;
    matrix[a][b] = true;
  }
}

function reserveFormatInfo(matrix) {
  for (let index = 0; index <= 5; index += 1) {
    matrix[index][8] = true;
    matrix[8][index] = true;
  }
  matrix[7][8] = true;
  matrix[8][8] = true;
  matrix[8][7] = true;

  for (let index = 9; index < 15; index += 1) {
    matrix[8][14 - index] = true;
  }
  for (let index = 0; index < 8; index += 1) {
    matrix[8][QR_SIZE - 1 - index] = true;
  }
  for (let index = 8; index < 15; index += 1) {
    matrix[QR_SIZE - 15 + index][8] = true;
  }
}

function versionBits() {
  let remainder = QR_VERSION << 12;
  const generator = 0x1f25;
  for (let bit = 17; bit >= 12; bit -= 1) {
    if (((remainder >>> bit) & 1) !== 0) {
      remainder ^= generator << (bit - 12);
    }
  }
  return (QR_VERSION << 12) | remainder;
}

function maskApplies(mask, x, y) {
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

function isDark(binary, width, x, y) {
  return binary[y * width + x] === 1;
}

function distanceSquared(left, right) {
  const dx = left.x - right.x;
  const dy = left.y - right.y;
  return dx * dx + dy * dy;
}

function crossProduct(origin, first, second) {
  return (first.x - origin.x) * (second.y - origin.y) - (first.y - origin.y) * (second.x - origin.x);
}

function sum(values) {
  return values.reduce((total, value) => total + value, 0);
}

function inQrBounds(x, y) {
  return x >= 0 && y >= 0 && x < QR_SIZE && y < QR_SIZE;
}
