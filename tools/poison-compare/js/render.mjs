// Renders SignalRGB's original Poison effect.html headless, N runs with different seeds,
// and writes the raw RGB pixels of the frame after the last step to one binary file.
// The Rust side (tools/poison-compare/src) computes every statistic for both sides, so both
// sides go through the same code.
//
// Usage:
//   node render.mjs --out <file.rgb> [--runs 20] [--frames 600] [--effect <effect.html>]
//                   [--bg #2d0075] [--color1 #6000fc] [--color2 #b300ff] [--color3 #8a42ff]
//                   [--speed 35]
//
// Output file: runs * 320 * 200 * 3 bytes, run after run, row-major RGB.
// stdout: one JSON line with the run count, seeds and the lowest pixel alpha seen.
// Never runs `playwright install`; uses the cached chromium_headless_shell-1248.

import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const WIDTH = 320;
const HEIGHT = 200;

function parseArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i += 2) {
    const key = argv[i];
    if (!key.startsWith('--')) throw new Error(`unexpected argument ${key}`);
    if (i + 1 >= argv.length) throw new Error(`missing value for ${key}`);
    args[key.slice(2)] = argv[i + 1];
  }
  return args;
}

const args = parseArgs(process.argv.slice(2));
const defaultEffect = path.join(
  process.env.LOCALAPPDATA ?? '',
  'WhirlwindFX', 'SignalRgb', 'cache', 'effects', '-Mir7bKkFQmd2LF_9Leg', 'effect.html',
);

const runs = Number(args.runs ?? 20);
const frames = Number(args.frames ?? 600);
const outPath = args.out;
const effectPath = args.effect ?? defaultEffect;
const props = {
  bg: args.bg ?? '#2d0075',
  color1: args.color1 ?? '#6000fc',
  color2: args.color2 ?? '#b300ff',
  color3: args.color3 ?? '#8a42ff',
  speed: Number(args.speed ?? 35),
};

if (!outPath) throw new Error('--out <file> is required');
if (!Number.isInteger(runs) || runs < 1) throw new Error('--runs must be a positive integer');
if (!Number.isInteger(frames) || frames < 1) throw new Error('--frames must be a positive integer');

// Runs before any page script. Seeds Math.random and replaces requestAnimationFrame with a
// slot that keeps only the latest callback, so the page advances one step per call.
function initScript(seed) {
  let s = seed >>> 0;
  Math.random = function () {
    s = (s + 0x6d2b79f5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  window.__rafCb = null;
  window.requestAnimationFrame = function (cb) {
    window.__rafCb = cb;
    return 1;
  };
  window.cancelAnimationFrame = function () {};
}

// Runs after the page script. Sets the SignalRGB globals, steps `frames` times, reads pixels.
function stepAndRead({ frames, props }) {
  window.bgColor = props.bg;
  window.color1 = props.color1;
  window.color2 = props.color2;
  window.color3 = props.color3;
  window.speedRaw = props.speed;

  for (let i = 0; i < frames; i++) {
    const cb = window.__rafCb;
    window.__rafCb = null;
    if (typeof cb !== 'function') throw new Error(`no requestAnimationFrame callback at step ${i}`);
    cb(i * (1000 / 60));
  }

  const canvas = document.getElementById('exCanvas');
  const data = canvas.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data;
  const rgb = new Uint8Array((data.length / 4) * 3);
  let minAlpha = 255;
  for (let p = 0, q = 0; p < data.length; p += 4, q += 3) {
    rgb[q] = data[p];
    rgb[q + 1] = data[p + 1];
    rgb[q + 2] = data[p + 2];
    if (data[p + 3] < minAlpha) minAlpha = data[p + 3];
  }
  let binary = '';
  for (let i = 0; i < rgb.length; i += 0x8000) {
    binary += String.fromCharCode.apply(null, rgb.subarray(i, i + 0x8000));
  }
  return { width: canvas.width, height: canvas.height, rgbBase64: btoa(binary), minAlpha };
}

const browser = await chromium.launch({ headless: true });
const chunks = [];
const seeds = [];
let minAlphaAll = 255;
let width = WIDTH;
let height = HEIGHT;

try {
  for (let run = 0; run < runs; run++) {
    const seed = 1000 + run;
    seeds.push(seed);
    const context = await browser.newContext({ viewport: { width: WIDTH, height: HEIGHT } });
    const page = await context.newPage();
    try {
      await page.addInitScript(initScript, seed);
      await page.goto(pathToFileURL(effectPath).href, { waitUntil: 'load' });
      const result = await page.evaluate(stepAndRead, { frames, props });
      if (result.width !== WIDTH || result.height !== HEIGHT) {
        throw new Error(`canvas is ${result.width}x${result.height}, expected ${WIDTH}x${HEIGHT}`);
      }
      width = result.width;
      height = result.height;
      minAlphaAll = Math.min(minAlphaAll, result.minAlpha);
      chunks.push(Buffer.from(result.rgbBase64, 'base64'));
      process.stderr.write(`run ${run + 1}/${runs} seed ${seed} done (min alpha ${result.minAlpha})\n`);
    } finally {
      await context.close();
    }
  }
} finally {
  await browser.close();
}

const all = Buffer.concat(chunks);
const expected = runs * width * height * 3;
if (all.length !== expected) throw new Error(`wrote ${all.length} bytes, expected ${expected}`);
fs.writeFileSync(outPath, all);

process.stdout.write(
  JSON.stringify({
    runs,
    frames,
    width,
    height,
    seeds,
    props,
    minAlpha: minAlphaAll,
    out: outPath,
    effect: effectPath,
  }) + '\n',
);
