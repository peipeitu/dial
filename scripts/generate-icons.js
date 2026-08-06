const fs = require("fs");
const path = require("path");
const zlib = require("zlib");

const root = path.resolve(__dirname, "..");
const buildDir = path.join(root, "build");
const linuxDir = path.join(buildDir, "icons");

const background = [255, 255, 255, 255];
const ink = [38, 49, 56, 255];
const mint = [69, 201, 172, 255];

fs.mkdirSync(buildDir, { recursive: true });
fs.mkdirSync(linuxDir, { recursive: true });

function clamp(value, min = 0, max = 1) {
  return Math.max(min, Math.min(max, value));
}

function roundedRectAlpha(px, py, x, y, width, height, radius) {
  const cx = x + width / 2;
  const cy = y + height / 2;
  const qx = Math.abs(px - cx) - (width / 2 - radius);
  const qy = Math.abs(py - cy) - (height / 2 - radius);
  const outside = Math.hypot(Math.max(qx, 0), Math.max(qy, 0));
  const inside = Math.min(Math.max(qx, qy), 0);
  const distance = outside + inside - radius;
  return clamp(0.5 - distance);
}

function circleAlpha(px, py, cx, cy, radius) {
  return clamp(0.5 - (Math.hypot(px - cx, py - cy) - radius));
}

function lineAlpha(px, py, x1, y1, x2, y2, width) {
  const vx = x2 - x1;
  const vy = y2 - y1;
  const lengthSq = vx * vx + vy * vy;
  const t = clamp(((px - x1) * vx + (py - y1) * vy) / lengthSq);
  const x = x1 + vx * t;
  const y = y1 + vy * t;
  return clamp(0.5 - (Math.hypot(px - x, py - y) - width / 2));
}

function composite(data, width, x, y, color, alpha) {
  if (alpha <= 0) {
    return;
  }
  const index = (y * width + x) * 4;
  const sourceAlpha = (color[3] / 255) * alpha;
  const targetAlpha = data[index + 3] / 255;
  const outAlpha = sourceAlpha + targetAlpha * (1 - sourceAlpha);
  if (outAlpha <= 0) {
    return;
  }
  data[index] = Math.round((color[0] * sourceAlpha + data[index] * targetAlpha * (1 - sourceAlpha)) / outAlpha);
  data[index + 1] = Math.round((color[1] * sourceAlpha + data[index + 1] * targetAlpha * (1 - sourceAlpha)) / outAlpha);
  data[index + 2] = Math.round((color[2] * sourceAlpha + data[index + 2] * targetAlpha * (1 - sourceAlpha)) / outAlpha);
  data[index + 3] = Math.round(outAlpha * 255);
}

function drawRoundedRect(data, size, rect, color) {
  const scale = size / 1024;
  const x = Math.floor(rect.x * scale);
  const y = Math.floor(rect.y * scale);
  const width = Math.ceil(rect.width * scale);
  const height = Math.ceil(rect.height * scale);
  const radius = rect.radius * scale;
  for (let py = Math.max(0, y - 2); py < Math.min(size, y + height + 2); py += 1) {
    for (let px = Math.max(0, x - 2); px < Math.min(size, x + width + 2); px += 1) {
      composite(data, size, px, py, color, roundedRectAlpha(px + 0.5, py + 0.5, x, y, width, height, radius));
    }
  }
}

function drawCircle(data, size, circle, color) {
  const scale = size / 1024;
  const cx = circle.cx * scale;
  const cy = circle.cy * scale;
  const radius = circle.radius * scale;
  const minX = Math.max(0, Math.floor(cx - radius - 2));
  const maxX = Math.min(size, Math.ceil(cx + radius + 2));
  const minY = Math.max(0, Math.floor(cy - radius - 2));
  const maxY = Math.min(size, Math.ceil(cy + radius + 2));
  for (let py = minY; py < maxY; py += 1) {
    for (let px = minX; px < maxX; px += 1) {
      composite(data, size, px, py, color, circleAlpha(px + 0.5, py + 0.5, cx, cy, radius));
    }
  }
}

function drawLine(data, size, line, color) {
  const scale = size / 1024;
  const x1 = line.x1 * scale;
  const y1 = line.y1 * scale;
  const x2 = line.x2 * scale;
  const y2 = line.y2 * scale;
  const width = line.width * scale;
  const minX = Math.max(0, Math.floor(Math.min(x1, x2) - width));
  const maxX = Math.min(size, Math.ceil(Math.max(x1, x2) + width));
  const minY = Math.max(0, Math.floor(Math.min(y1, y2) - width));
  const maxY = Math.min(size, Math.ceil(Math.max(y1, y2) + width));
  for (let py = minY; py < maxY; py += 1) {
    for (let px = minX; px < maxX; px += 1) {
      composite(data, size, px, py, color, lineAlpha(px + 0.5, py + 0.5, x1, y1, x2, y2, width));
    }
  }
}

function drawArc(data, size, arc, color) {
  const sweep = Math.abs(arc.endDegrees - arc.startDegrees);
  const steps = Math.max(1, Math.ceil(sweep * 1.5));
  for (let step = 0; step <= steps; step += 1) {
    const progress = step / steps;
    const degrees = arc.startDegrees + (arc.endDegrees - arc.startDegrees) * progress;
    const radians = degrees * Math.PI / 180;
    drawCircle(
      data,
      size,
      {
        cx: arc.cx + arc.radius * Math.cos(radians),
        cy: arc.cy + arc.radius * Math.sin(radians),
        radius: arc.width / 2,
      },
      color,
    );
  }
}

function renderIcon(size) {
  const data = Buffer.alloc(size * size * 4);
  drawRoundedRect(data, size, { x: 64, y: 64, width: 896, height: 896, radius: 218 }, background);
  drawArc(data, size, { cx: 512, cy: 580, radius: 294, width: 64, startDegrees: 166, endDegrees: 220 }, ink);
  drawArc(data, size, { cx: 512, cy: 580, radius: 294, width: 64, startDegrees: 246, endDegrees: 294 }, ink);
  drawArc(data, size, { cx: 512, cy: 580, radius: 294, width: 68, startDegrees: 320, endDegrees: 375 }, mint);
  drawLine(data, size, { x1: 512, y1: 594, x2: 666, y2: 440, width: 64 }, mint);
  drawCircle(data, size, { cx: 512, cy: 594, radius: 67 }, mint);
  return data;
}

function crc32(buffer) {
  let crc = ~0;
  for (const byte of buffer) {
    crc ^= byte;
    for (let index = 0; index < 8; index += 1) {
      crc = crc & 1 ? 0xedb88320 ^ (crc >>> 1) : crc >>> 1;
    }
  }
  return ~crc >>> 0;
}

function chunk(type, data) {
  const typeBuffer = Buffer.from(type);
  const length = Buffer.alloc(4);
  const crc = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuffer, data])));
  return Buffer.concat([length, typeBuffer, data, crc]);
}

function pngBuffer(width, height, rgba) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8;
  header[9] = 6;
  const scanlines = Buffer.alloc((width * 4 + 1) * height);
  for (let y = 0; y < height; y += 1) {
    const sourceStart = y * width * 4;
    const targetStart = y * (width * 4 + 1) + 1;
    rgba.copy(scanlines, targetStart, sourceStart, sourceStart + width * 4);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", zlib.deflateSync(scanlines, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

function writePng(target, size) {
  fs.writeFileSync(target, pngBuffer(size, size, renderIcon(size)));
}

function writeIco(target, sizes) {
  const images = sizes.map((size) => ({ size, data: pngBuffer(size, size, renderIcon(size)) }));
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(images.length, 4);
  let offset = 6 + images.length * 16;
  const entries = images.map(({ size, data }) => {
    const entry = Buffer.alloc(16);
    entry[0] = size >= 256 ? 0 : size;
    entry[1] = size >= 256 ? 0 : size;
    entry[2] = 0;
    entry[3] = 0;
    entry.writeUInt16LE(1, 4);
    entry.writeUInt16LE(32, 6);
    entry.writeUInt32LE(data.length, 8);
    entry.writeUInt32LE(offset, 12);
    offset += data.length;
    return entry;
  });
  fs.writeFileSync(target, Buffer.concat([header, ...entries, ...images.map((image) => image.data)]));
}

function writeIcns(target, sizeMap) {
  const chunks = Object.entries(sizeMap).map(([type, size]) => {
    const data = pngBuffer(size, size, renderIcon(size));
    const header = Buffer.alloc(8);
    header.write(type, 0, 4, "ascii");
    header.writeUInt32BE(data.length + 8, 4);
    return Buffer.concat([header, data]);
  });
  const totalLength = 8 + chunks.reduce((total, item) => total + item.length, 0);
  const header = Buffer.alloc(8);
  header.write("icns", 0, 4, "ascii");
  header.writeUInt32BE(totalLength, 4);
  fs.writeFileSync(target, Buffer.concat([header, ...chunks]));
}

writePng(path.join(buildDir, "icon.png"), 1024);
for (const size of [16, 20, 24, 28, 32, 36, 40, 48, 56, 64, 72, 80, 96, 112, 128, 256, 512, 1024]) {
  writePng(path.join(linuxDir, `${size}x${size}.png`), size);
}

writeIcns(path.join(buildDir, "icon.icns"), {
  icp4: 16,
  icp5: 32,
  icp6: 64,
  ic07: 128,
  ic08: 256,
  ic09: 512,
  ic10: 1024,
});
writeIco(path.join(buildDir, "icon.ico"), [16, 32, 48, 64, 128, 256]);
