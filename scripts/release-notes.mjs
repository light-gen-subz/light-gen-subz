// Prints the GitHub release notes for a version: its section of CHANGELOG.md and of
// CHANGELOG.fr.md, one after the other.
//
//   node scripts/release-notes.mjs v1.5.0 > notes.md
//
// Fails (exit 1) when either changelog has no section for that version, so a release can
// never go out without notes in both languages.

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

export function section(text, version) {
  const lines = String(text).split(/\r?\n/);
  const start = lines.findIndex((l) =>
    new RegExp(`^##\\s+\\[?v?${version.replace(/\./g, '\\.')}\\]?(\\s|$)`).test(l),
  );
  if (start < 0) return null;
  let end = lines.findIndex((l, i) => i > start && /^##\s/.test(l));
  if (end < 0) end = lines.length;
  return lines.slice(start + 1, end).join('\n').trim();
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const version = String(process.argv[2] || '').replace(/^v/, '');
  if (!version) {
    console.error('usage: release-notes.mjs <version>');
    process.exit(2);
  }
  const root = resolve(import.meta.dirname, '..');
  const part = (file, title) => {
    const body = section(readFileSync(resolve(root, file), 'utf8'), version);
    if (!body) {
      console.error(`${file} has no section for ${version}`);
      process.exit(1);
    }
    return `## ${title}\n\n${body.replace(/^### /gm, '#### ')}`;
  };
  console.log([part('CHANGELOG.md', 'English'), part('CHANGELOG.fr.md', 'Français')].join('\n\n---\n\n'));
}
