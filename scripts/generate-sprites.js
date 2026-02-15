// Generate simple pixel-art thronglet sprites as PNG files
// Uses raw PNG encoding (no dependencies needed)

const fs = require('fs');
const path = require('path');
const zlib = require('zlib');

const SIZE = 40;
const ASSETS_DIR = path.join(__dirname, '..', 'public', 'assets');

function createPNG(width, height, pixels) {
  // PNG file structure
  const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

  function chunk(type, data) {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const typeB = Buffer.from(type);
    const crc = crc32(Buffer.concat([typeB, data]));
    const crcB = Buffer.alloc(4);
    crcB.writeUInt32BE(crc >>> 0);
    return Buffer.concat([len, typeB, data, crcB]);
  }

  // IHDR
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  ihdr[10] = 0; // compression
  ihdr[11] = 0; // filter
  ihdr[12] = 0; // interlace

  // IDAT - raw pixel data with filter bytes
  const rawData = Buffer.alloc(height * (1 + width * 4));
  for (let y = 0; y < height; y++) {
    rawData[y * (1 + width * 4)] = 0; // no filter
    for (let x = 0; x < width; x++) {
      const pi = (y * width + x) * 4;
      const offset = y * (1 + width * 4) + 1 + x * 4;
      rawData[offset] = pixels[pi];     // R
      rawData[offset + 1] = pixels[pi + 1]; // G
      rawData[offset + 2] = pixels[pi + 2]; // B
      rawData[offset + 3] = pixels[pi + 3]; // A
    }
  }

  const compressed = zlib.deflateSync(rawData);

  // IEND
  const iend = Buffer.alloc(0);

  return Buffer.concat([
    signature,
    chunk('IHDR', ihdr),
    chunk('IDAT', compressed),
    chunk('IEND', iend),
  ]);
}

// CRC32 lookup table
const crcTable = (function () {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) {
      if (c & 1) c = 0xedb88320 ^ (c >>> 1);
      else c = c >>> 1;
    }
    table[n] = c;
  }
  return table;
})();

function crc32(buf) {
  let crc = 0xffffffff;
  for (let i = 0; i < buf.length; i++) {
    crc = crcTable[(crc ^ buf[i]) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function setPixel(pixels, w, x, y, r, g, b, a = 255) {
  const i = (y * w + x) * 4;
  pixels[i] = r;
  pixels[i + 1] = g;
  pixels[i + 2] = b;
  pixels[i + 3] = a;
}

function fillRect(pixels, w, x1, y1, x2, y2, r, g, b, a = 255) {
  for (let y = y1; y <= y2; y++) {
    for (let x = x1; x <= x2; x++) {
      setPixel(pixels, w, x, y, r, g, b, a);
    }
  }
}

function drawCircle(pixels, w, cx, cy, radius, r, g, b, a = 255) {
  for (let y = cy - radius; y <= cy + radius; y++) {
    for (let x = cx - radius; x <= cx + radius; x++) {
      const dx = x - cx;
      const dy = y - cy;
      if (dx * dx + dy * dy <= radius * radius) {
        if (x >= 0 && x < w && y >= 0 && y < SIZE) {
          setPixel(pixels, w, x, y, r, g, b, a);
        }
      }
    }
  }
}

// --- IDLE sprite: happy blob with smile ---
function drawIdle() {
  const pixels = new Uint8Array(SIZE * SIZE * 4); // starts transparent

  // Body - rounded green blob
  drawCircle(pixels, SIZE, 20, 24, 14, 0x4F, 0xC0, 0x8D); // green body

  // Eyes - white with black pupils
  drawCircle(pixels, SIZE, 14, 20, 3, 255, 255, 255); // left eye white
  drawCircle(pixels, SIZE, 26, 20, 3, 255, 255, 255); // right eye white
  setPixel(pixels, SIZE, 14, 20, 0, 0, 0);  // left pupil
  setPixel(pixels, SIZE, 15, 20, 0, 0, 0);
  setPixel(pixels, SIZE, 26, 20, 0, 0, 0);  // right pupil
  setPixel(pixels, SIZE, 27, 20, 0, 0, 0);

  // Smile
  for (let x = 15; x <= 25; x++) {
    const y = 28 + Math.round(Math.pow((x - 20) / 5, 2) * 2);
    if (y < SIZE) setPixel(pixels, SIZE, x, y, 0, 0, 0);
  }

  // Little feet
  fillRect(pixels, SIZE, 12, 37, 16, 39, 0x3A, 0x90, 0x68);
  fillRect(pixels, SIZE, 24, 37, 28, 39, 0x3A, 0x90, 0x68);

  return pixels;
}

// --- WALKING sprite: blob mid-step ---
function drawWalking() {
  const pixels = new Uint8Array(SIZE * SIZE * 4);

  // Body - orange/amber blob (busy = working)
  drawCircle(pixels, SIZE, 20, 22, 14, 0xF5, 0xA6, 0x23);

  // Eyes - focused (slightly squinted)
  fillRect(pixels, SIZE, 12, 19, 17, 21, 255, 255, 255);
  fillRect(pixels, SIZE, 24, 19, 29, 21, 255, 255, 255);
  setPixel(pixels, SIZE, 15, 20, 0, 0, 0);
  setPixel(pixels, SIZE, 16, 20, 0, 0, 0);
  setPixel(pixels, SIZE, 26, 20, 0, 0, 0);
  setPixel(pixels, SIZE, 27, 20, 0, 0, 0);

  // Determined mouth - straight line
  for (let x = 16; x <= 24; x++) {
    setPixel(pixels, SIZE, x, 28, 0, 0, 0);
  }

  // Walking feet - one forward, one back
  fillRect(pixels, SIZE, 8, 35, 13, 39, 0xC0, 0x80, 0x1A);
  fillRect(pixels, SIZE, 26, 35, 31, 39, 0xC0, 0x80, 0x1A);

  // Motion lines
  setPixel(pixels, SIZE, 5, 22, 0xF5, 0xA6, 0x23, 150);
  setPixel(pixels, SIZE, 4, 22, 0xF5, 0xA6, 0x23, 100);
  setPixel(pixels, SIZE, 5, 25, 0xF5, 0xA6, 0x23, 150);
  setPixel(pixels, SIZE, 4, 25, 0xF5, 0xA6, 0x23, 100);

  return pixels;
}

// --- SAD sprite: error state, red and frowning ---
function drawSad() {
  const pixels = new Uint8Array(SIZE * SIZE * 4);

  // Body - reddish blob
  drawCircle(pixels, SIZE, 20, 24, 14, 0xE7, 0x4C, 0x3C);

  // Eyes - X eyes for error
  // Left X
  setPixel(pixels, SIZE, 12, 18, 0, 0, 0);
  setPixel(pixels, SIZE, 13, 19, 0, 0, 0);
  setPixel(pixels, SIZE, 14, 20, 0, 0, 0);
  setPixel(pixels, SIZE, 15, 21, 0, 0, 0);
  setPixel(pixels, SIZE, 16, 22, 0, 0, 0);
  setPixel(pixels, SIZE, 16, 18, 0, 0, 0);
  setPixel(pixels, SIZE, 15, 19, 0, 0, 0);
  setPixel(pixels, SIZE, 13, 21, 0, 0, 0);
  setPixel(pixels, SIZE, 12, 22, 0, 0, 0);

  // Right X
  setPixel(pixels, SIZE, 24, 18, 0, 0, 0);
  setPixel(pixels, SIZE, 25, 19, 0, 0, 0);
  setPixel(pixels, SIZE, 26, 20, 0, 0, 0);
  setPixel(pixels, SIZE, 27, 21, 0, 0, 0);
  setPixel(pixels, SIZE, 28, 22, 0, 0, 0);
  setPixel(pixels, SIZE, 28, 18, 0, 0, 0);
  setPixel(pixels, SIZE, 27, 19, 0, 0, 0);
  setPixel(pixels, SIZE, 25, 21, 0, 0, 0);
  setPixel(pixels, SIZE, 24, 22, 0, 0, 0);

  // Frown
  for (let x = 15; x <= 25; x++) {
    const y = 30 - Math.round(Math.pow((x - 20) / 5, 2) * 2);
    if (y >= 0) setPixel(pixels, SIZE, x, y, 0, 0, 0);
  }

  // Droopy feet
  fillRect(pixels, SIZE, 14, 37, 18, 39, 0xB0, 0x3A, 0x2E);
  fillRect(pixels, SIZE, 22, 37, 26, 39, 0xB0, 0x3A, 0x2E);

  return pixels;
}

// Generate all sprites
fs.mkdirSync(ASSETS_DIR, { recursive: true });

const sprites = {
  'idle.png': drawIdle(),
  'walking.png': drawWalking(),
  'sad.png': drawSad(),
};

for (const [name, pixels] of Object.entries(sprites)) {
  const png = createPNG(SIZE, SIZE, pixels);
  const outPath = path.join(ASSETS_DIR, name);
  fs.writeFileSync(outPath, png);
  console.log(`Created ${outPath} (${png.length} bytes)`);
}

console.log('Done!');
