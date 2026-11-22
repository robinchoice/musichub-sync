import { expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

// WCAG contrast of the colour pairs the app draws. Values come from where they are
// defined: the tokens in app.css and the avatar colours in Popover.svelte. The app
// has only a dark theme. Text needs 4.5:1, the focus ring 3:1.

const css = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8');
const popover = readFileSync(new URL('../src/lib/Popover.svelte', import.meta.url), 'utf8');

const tokens = Object.fromEntries([...css.matchAll(/^\s*(--[\w-]+): (#[0-9a-f]{6});$/gim)].map(([, name, hex]) => [name, hex]));
const gradient = css.match(/--grad: linear-gradient\((.+)\);/)?.[1].match(/#[0-9a-f]{6}/gi) ?? [];
const avatars = popover.match(/const COLORS = \[(.+)\];/)?.[1].match(/#[0-9a-f]{6}/gi) ?? [];

function luminance(hex) {
  const [r, g, b] = [1, 3, 5].map((i) => {
    const v = Number.parseInt(hex.slice(i, i + 2), 16) / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a, b) {
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (light + 0.05) / (dark + 0.05);
}

// Lists every failing pair, so one run shows all of them
function failures(pairs) {
  return pairs.flatMap(([fg, bg, minimum]) => {
    const [a, b] = [tokens[fg] ?? fg, tokens[bg] ?? bg];
    if (!/^#[0-9a-f]{6}$/i.test(a) || !/^#[0-9a-f]{6}$/i.test(b)) return [`${fg} on ${bg}: not a hex colour`];
    const ratio = contrast(a, b);
    return ratio < minimum ? [`${fg} ${a} on ${bg} ${b}: ${ratio.toFixed(2)} < ${minimum}`] : [];
  });
}

test('colours are read from their sources', () => {
  expect(Object.keys(tokens).length).toBeGreaterThan(5);
  expect(gradient.length).toBeGreaterThan(1);
  expect(avatars.length).toBeGreaterThan(1);
});

test('text and focus ring keep their contrast', () => {
  const pairs = [];
  for (const bg of ['--bg', '--raised']) {
    for (const fg of ['--text', '--muted', '--needs', '--new', '--error']) pairs.push([fg, bg, 4.5]);
    pairs.push(['--accent', bg, 3]);
  }
  // Hovered icon buttons
  pairs.push(['--text', '--overlay', 4.5]);
  // Primary buttons on the gradient stops, initials on the avatar colours and the error avatar
  for (const bg of [...gradient, ...avatars, '--error']) pairs.push(['--on-grad', bg, 4.5]);
  expect(failures(pairs)).toEqual([]);
});
