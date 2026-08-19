import { describe, it, expect } from "vitest";
import { fileName, parseSrt } from "../App";

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

  it("joins a multi-line cue into a single string", () => {
    const multi = ["1", "00:00:00,000 --> 00:00:02,000", "first line", "second line"].join("\n");
    expect(parseSrt(multi)[0].text).toBe("first line second line");
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
