// CHANGELOG.md (English) and CHANGELOG.fr.md (French) follow "Keep a Changelog":
//
//   ## [1.0.2] - 2026-10-21
//   ### Fixed
//   - …
//
// This module reads them. The same entries feed the update banner and the in-app
// "What's new" window, and the release notes on GitHub.

import { isNewer } from "./version";

export interface ChangelogEntry {
  version: string;
  date: string;
  body: string;
}

/** Entries in file order (newest first). A section that is not a version, such as
 * "## [Unreleased]", is skipped. */
export function parseChangelog(text: string): ChangelogEntry[] {
  const entries: { version: string; date: string; lines: string[] }[] = [];
  let current: (typeof entries)[number] | null = null;
  for (const line of String(text).split(/\r?\n/)) {
    const head = /^##\s+\[?v?(\d+\.\d+\.\d+[0-9A-Za-z.+-]*)\]?(?:\s+[-–—]\s+(\S+))?/.exec(line);
    if (head) {
      current = { version: head[1], date: head[2] || "", lines: [] };
      entries.push(current);
    } else if (/^##\s/.test(line)) {
      current = null;
    } else if (current) {
      current.lines.push(line);
    }
  }
  return entries.map((e) => ({
    version: e.version,
    date: e.date,
    body: e.lines.join("\n").trim(),
  }));
}

/** Entries newer than `current` and no newer than `latest`: what an update brings. */
export function entriesBetween(entries: ChangelogEntry[], current: string, latest: string) {
  return entries.filter((e) => isNewer(e.version, current) && !isNewer(e.version, latest));
}

/** Entries that already shipped in `current` or earlier: what the app shows as history. */
export function entriesUpTo(entries: ChangelogEntry[], current: string) {
  return entries.filter((e) => !isNewer(e.version, current));
}

/** Plain text for the update banner: headings become "Fixed:", bullets become "•".
 * Long notes are cut at a line boundary. */
export function formatNotes(entries: ChangelogEntry[], maxChars = 1500): string {
  const out: string[] = [];
  for (const e of entries) {
    out.push(`${e.version}${e.date ? ` (${e.date})` : ""}`);
    for (const raw of e.body.split("\n")) {
      const line = raw.trim();
      if (!line) continue;
      const heading = /^###\s+(.*)$/.exec(line);
      if (heading) out.push(`${heading[1]}:`);
      else
        out.push(
          line
            .replace(/^[-*]\s+/, "• ")
            .replace(/\*\*(.+?)\*\*/g, "$1")
            .replace(/\*(.+?)\*/g, "$1")
            .replace(/`([^`]+)`/g, "$1"),
        );
    }
    out.push("");
  }
  const text = out.join("\n").trim();
  if (text.length <= maxChars) return text;
  const cut = text.slice(0, maxChars);
  const at = cut.lastIndexOf("\n");
  return `${cut.slice(0, at > 0 ? at : maxChars)}\n…`;
}
