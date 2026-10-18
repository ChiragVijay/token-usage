const fs = require('node:fs/promises');
const path = require('node:path');
const sharp = require('sharp');

const assets = path.resolve(__dirname, '../assets');

async function png(size) {
  return sharp(path.join(assets, 'AppIcon.svg')).resize(size, size).png().toBuffer();
}

async function generate() {
  await fs.writeFile(path.join(assets, 'AppIcon.png'), await png(1024));
  const sizes = [16, 24, 32, 48, 64, 128, 256];
  const images = await Promise.all(sizes.map(png));
  const header = Buffer.alloc(6 + sizes.length * 16);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(sizes.length, 4);
  let offset = header.length;
  images.forEach((image, index) => {
    const entry = 6 + index * 16;
    header[entry] = sizes[index] % 256;
    header[entry + 1] = sizes[index] % 256;
    header.writeUInt16LE(1, entry + 4);
    header.writeUInt16LE(32, entry + 6);
    header.writeUInt32LE(image.length, entry + 8);
    header.writeUInt32LE(offset, entry + 12);
    offset += image.length;
  });
  await fs.writeFile(path.join(assets, 'AppIcon.ico'), Buffer.concat([header, ...images]));
  const chunks = [];
  for (const [size, type] of [[16, 'icp4'], [32, 'icp5'], [64, 'icp6'], [128, 'ic07'], [256, 'ic08'], [512, 'ic09'], [1024, 'ic10']]) {
    const image = await png(size);
    const chunk = Buffer.alloc(8);
    chunk.write(type);
    chunk.writeUInt32BE(image.length + 8, 4);
    chunks.push(chunk, image);
  }
  const container = Buffer.alloc(8);
  container.write('icns');
  container.writeUInt32BE(8 + chunks.reduce((sum, chunk) => sum + chunk.length, 0), 4);
  await fs.writeFile(path.join(assets, 'AppIcon.icns'), Buffer.concat([container, ...chunks]));
  for (const [source, output] of [['AppIcon.svg', 'TrayIcon.rgba'], ['TrayIcon.svg', 'TrayIconTemplate.rgba']]) {
    const pixels = await sharp(path.join(assets, source)).resize(32, 32).ensureAlpha().raw().toBuffer();
    await fs.writeFile(path.join(assets, output), pixels);
  }
}

generate().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
