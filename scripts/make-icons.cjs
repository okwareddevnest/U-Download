#!/usr/bin/env node
// Generate square 256x256 and 512x512 icons for Tauri bundler
// Uses src-tauri/icons/icon.png (preferred) or ./logo.png as source

const fs = require('fs');
const path = require('path');
const Jimp = require('jimp');

async function ensureDir(p) {
  await fs.promises.mkdir(p, { recursive: true });
}

async function createSquareIcon(srcPath, outPath, size) {
  const img = await Jimp.read(srcPath);
  const { width, height } = img.bitmap;
  const scale = Math.min(size / width, size / height);
  const nw = Math.max(1, Math.round(width * scale));
  const nh = Math.max(1, Math.round(height * scale));

  const resized = img.clone().resize(nw, nh, Jimp.RESIZE_BILINEAR);
  const canvas = new Jimp(size, size, 0x00000000);
  const x = Math.floor((size - nw) / 2);
  const y = Math.floor((size - nh) / 2);
  canvas.composite(resized, x, y);
  await canvas.write(outPath);
  console.log(`✓ Wrote ${outPath}`);
}

(async () => {
  try {
    const iconsDir = path.join(__dirname, '..', 'src-tauri', 'icons');
    const srcCandidates = [
      path.join(iconsDir, 'icon.png'),
      path.join(__dirname, '..', 'logo.png'),
    ];
    let src = null;
    for (const p of srcCandidates) {
      if (fs.existsSync(p)) { src = p; break; }
    }
    if (!src) {
      console.error('No source image found. Expected src-tauri/icons/icon.png or ./logo.png');
      process.exit(1);
    }

    await ensureDir(iconsDir);
    await createSquareIcon(src, path.join(iconsDir, '256x256.png'), 256);
    await createSquareIcon(src, path.join(iconsDir, '512x512.png'), 512);
  } catch (e) {
    console.error(e);
    process.exit(1);
  }
})();

