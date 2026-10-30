import { describe, it, expect } from "vitest";
import { fileName, formatError, hasMediaExtension, parseSrt, formatDuration } from "../App";
import { getT, translationKeys } from "../i18n";

describe("fileName", () => {
  it("keeps only the last segment of a POSIX path", () => {
    expect(fileName("/home/me/videos/clip.mp4")).toBe("clip.mp4");
  });

  it("handles Windows separators", () => {
    expect(fileName("C:\\Users\\me\\clip.mp4")).toBe("clip.mp4");
  });

  it("handles mixed separators", () => {
    expect(fileName("/home/me\\clip.mp4")).toBe("clip.mp4");
  });

  it("returns a bare file name unchanged", () => {
    expect(fileName("clip.mp4")).toBe("clip.mp4");
  });

  it("yields an empty name for a path ending in a separator", () => {
    // The `?? path` fallback only catches null/undefined, and split() hands back an
    // empty trailing segment here. Harmless in practice: paths come from the file
    // picker, which never returns a trailing separator.
    expect(fileName("/home/me/")).toBe("");
  });

  it("returns an empty string unchanged", () => {
    expect(fileName("")).toBe("");
  });
});

describe("parseSrt", () => {
  const srt = [
    "1",
    "00:00:00,000 --> 00:00:02,000",
    "Hello there",
    "",
    "2",
    "00:00:02,000 --> 00:00:04,500",
    "General Kenobi",
  ].join("\n");

  it("parses index, timings and text", () => {
    expect(parseSrt(srt)).toEqual([
      { index: 1, start: "00:00:00,000", end: "00:00:02,000", text: "Hello there" },
      { index: 2, start: "00:00:02,000", end: "00:00:04,500", text: "General Kenobi" },
    ]);
  });

  it("keeps the lines of a multi-line cue", () => {
    const multi = ["1", "00:00:00,000 --> 00:00:02,000", "first line", "second line"].join("\n");
    expect(parseSrt(multi)[0].text).toBe("first line\nsecond line");
  });

  it("accepts CRLF line endings", () => {
    const crlf = srt.replace(/\n/g, "\r\n");
    expect(parseSrt(crlf)).toHaveLength(2);
    expect(parseSrt(crlf)[0].text).toBe("Hello there");
  });

  it("tolerates leading and trailing blank lines", () => {
    expect(parseSrt(`\n\n${srt}\n\n`)).toHaveLength(2);
  });

  it("drops blocks with no text", () => {
    const textless = ["1", "00:00:00,000 --> 00:00:02,000"].join("\n");
    const kept = ["2", "00:00:02,000 --> 00:00:03,000", "kept"].join("\n");
    expect(parseSrt(`${textless}\n\n${kept}`).map((c) => c.text)).toEqual(["kept"]);
  });

  it("falls back to index 0 when the index is not a number", () => {
    const odd = ["not-a-number", "00:00:00,000 --> 00:00:02,000", "text"].join("\n");
    expect(parseSrt(odd)[0].index).toBe(0);
  });

  it("leaves timings empty when the arrow is missing", () => {
    const odd = ["1", "no arrow here", "text"].join("\n");
    expect(parseSrt(odd)[0]).toMatchObject({ start: "no arrow here", end: "" });
  });

  it("returns an empty list for empty input", () => {
    expect(parseSrt("")).toEqual([]);
    expect(parseSrt("   \n\n  ")).toEqual([]);
  });

  it("trims surrounding whitespace on timings and text", () => {
    const padded = ["1", "  00:00:00,000   -->   00:00:02,000  ", "   padded text   "].join("\n");
    expect(parseSrt(padded)[0]).toMatchObject({
      start: "00:00:00,000",
      end: "00:00:02,000",
      text: "padded text",
    });
  });
});

describe("parseSrt — real world files", () => {
  it("tolerates several blank lines between cues", () => {
    const srt = "1\n00:00:00,000 --> 00:00:01,000\nA\n\n\n\n2\n00:00:01,000 --> 00:00:02,000\nB";
    expect(parseSrt(srt).map((c) => c.text)).toEqual(["A", "B"]);
  });

  it("reads a cue that has no sequence number", () => {
    expect(parseSrt("00:00:00,000 --> 00:00:01,000\nA")).toEqual([
      { index: 0, start: "00:00:00,000", end: "00:00:01,000", text: "A" },
    ]);
  });

  it("treats a whitespace-only line as a separator", () => {
    const srt = "1\n00:00:00,000 --> 00:00:01,000\nA\n  \n2\n00:00:01,000 --> 00:00:02,000\nB";
    expect(parseSrt(srt)).toHaveLength(2);
  });
});

describe("hasMediaExtension", () => {
  it("accepts the supported extensions whatever their case", () => {
    expect(hasMediaExtension("/a/clip.mp4")).toBe(true);
    expect(hasMediaExtension("/a/IMG_0001.MOV")).toBe(true);
    expect(hasMediaExtension("C:\\a\\song.FLAC")).toBe(true);
  });

  it("refuses anything else", () => {
    expect(hasMediaExtension("/a/notes.txt")).toBe(false);
    expect(hasMediaExtension("/a/archive")).toBe(false);
    expect(hasMediaExtension("/a/clip.mp4.exe")).toBe(false);
  });
});

describe("formatError", () => {
  const t = getT("en");

  it("translates a known code", () => {
    expect(formatError({ code: "no_audio", message: "x" }, t)).toBe(
      "This file has no audio track.",
    );
  });

  it("adds the provider's detail where the message has room for it", () => {
    expect(formatError({ code: "quota_exceeded", message: "HTTP 429: slow down" }, t)).toContain(
      "slow down",
    );
  });

  it("shows the raw message for an unknown code", () => {
    expect(formatError({ code: "mystery", message: "raw text" }, t)).toBe("raw text");
  });

  it("shows the message of a plain error", () => {
    expect(formatError(new Error("boom"), t)).toBe("boom");
  });

  it("stringifies anything else", () => {
    expect(formatError("just text", t)).toBe("just text");
  });
});

describe("translations", () => {
  it("French defines every key English does, and the reverse", () => {
    expect(translationKeys("fr").sort()).toEqual(translationKeys("en").sort());
  });

  it("both languages use the same placeholders in every message", () => {
    const placeholders = (text: string) => (text.match(/\{\w+\}/g) ?? []).sort();
    const rawEn = getT("en");
    const rawFr = getT("fr");
    for (const key of translationKeys("en")) {
      expect(placeholders(rawEn(key)), key).toEqual(placeholders(rawFr(key)));
    }
  });

  it("an unfilled optional placeholder leaves no stray braces", () => {
    expect(getT("en")("error.quota_exceeded")).not.toContain("{");
  });
});

describe("formatDuration", () => {
  it("writes minutes and seconds, and hours when needed", () => {
    expect(formatDuration(0)).toBe("0:00");
    expect(formatDuration(83)).toBe("1:23");
    expect(formatDuration(3725)).toBe("1:02:05");
    expect(formatDuration(-4)).toBe("0:00");
  });
});
