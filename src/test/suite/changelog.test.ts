import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { entriesBetween, entriesUpTo, formatNotes, parseChangelog } from "../../suite/changelog";
import { isNewer } from "../../suite/version";

const SAMPLE = `# Changelog

## [Unreleased]
- not shipped

## [1.2.0] - 2026-03-01
### Added
- **Bold** feature with \`code\`

### Fixed
- a bug

## [1.1.0] - 2026-02-01
### Fixed
- older bug

## 1.0.0 - 2026-01-01
- first
`;

describe("isNewer", () => {
  it("compares numerically, not as text", () => {
    expect(isNewer("1.10.0", "1.9.0")).toBe(true);
    expect(isNewer("1.0.0", "1.0.0")).toBe(false);
    expect(isNewer("1.0.0", "1.0.1")).toBe(false);
    expect(isNewer("v2.0.0", "1.9.9")).toBe(true);
  });
});

describe("parseChangelog", () => {
  const entries = parseChangelog(SAMPLE);

  it("reads versions and dates, newest first, skipping Unreleased", () => {
    expect(entries.map((e) => [e.version, e.date])).toEqual([
      ["1.2.0", "2026-03-01"],
      ["1.1.0", "2026-02-01"],
      ["1.0.0", "2026-01-01"],
    ]);
  });
  it("keeps the body of each entry and does not leak neighbours", () => {
    expect(entries[0].body).toContain("### Added");
    expect(entries[0].body).toContain("- a bug");
    expect(entries[0].body).not.toContain("older bug");
    expect(entries.some((e) => e.body.includes("not shipped"))).toBe(false);
  });
  it("accepts a version without brackets or date, CRLF and empty input", () => {
    expect(parseChangelog("## v2.0.0\n- x")).toEqual([{ version: "2.0.0", date: "", body: "- x" }]);
    expect(parseChangelog("## [1.0.0] - 2026-01-01\r\n- a\r\n")[0].body).toBe("- a");
    expect(parseChangelog("")).toEqual([]);
  });
});

describe("selecting entries", () => {
  const entries = parseChangelog(SAMPLE);
  it("entriesBetween: what an update brings", () => {
    expect(entriesBetween(entries, "1.0.0", "1.2.0").map((e) => e.version)).toEqual([
      "1.2.0",
      "1.1.0",
    ]);
    expect(entriesBetween(entries, "1.0.0", "1.1.0").map((e) => e.version)).toEqual(["1.1.0"]);
    expect(entriesBetween(entries, "1.2.0", "1.2.0")).toEqual([]);
  });
  it("entriesUpTo: only what already shipped", () => {
    expect(entriesUpTo(entries, "1.1.0").map((e) => e.version)).toEqual(["1.1.0", "1.0.0"]);
  });
});

describe("formatNotes", () => {
  it("turns headings and bullets into plain text", () => {
    const text = formatNotes(parseChangelog(SAMPLE).slice(0, 1));
    expect(text).toContain("1.2.0 (2026-03-01)");
    expect(text).toContain("Added:");
    expect(text).toContain("• Bold feature with code");
    expect(text).not.toContain("**");
  });
  it("drops the emphasis marks", () => {
    const text = formatNotes(parseChangelog("## [1.0.0] - x\n- press *Cancel* or **Esc**"));
    expect(text).toContain("• press Cancel or Esc");
  });
  it("cuts long notes at a line boundary", () => {
    const long = parseChangelog(`## [1.0.0] - x\n${"- line\n".repeat(500)}`);
    const text = formatNotes(long, 100);
    expect(text.length).toBeLessThanOrEqual(104);
    expect(text.endsWith("…")).toBe(true);
  });
});

// The two changelogs and the version are kept in step by this test, not by memory.
describe("the project changelogs", () => {
  const root = resolve(import.meta.dirname, "../../..");
  const read = (f: string) => readFileSync(resolve(root, f), "utf8");
  const en = parseChangelog(read("CHANGELOG.md"));
  const fr = parseChangelog(read("CHANGELOG.fr.md"));
  const version = (JSON.parse(read("package.json")) as { version: string }).version;

  it("list the same versions in both languages", () => {
    expect(fr.map((e) => e.version)).toEqual(en.map((e) => e.version));
  });
  it("have notes for every version", () => {
    for (const e of [...en, ...fr]) expect(e.body.length, e.version).toBeGreaterThan(0);
  });
  it("describe the version in package.json", () => {
    expect(en.map((e) => e.version)).toContain(version);
    expect(en[0].version).toBe(version);
  });
});
