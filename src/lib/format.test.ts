import { describe, expect, it } from "vitest";
import { baseName, breadcrumbs, extensionOf, formatSize, parentOf } from "./format";

describe("formatSize", () => {
  it("フォルダは --", () => expect(formatSize(0, true)).toBe("--"));
  it("1000 未満はバイト", () => expect(formatSize(999, false)).toBe("999 バイト"));
  it("KB / MB に換算する", () => {
    expect(formatSize(1500, false)).toBe("1.5 KB");
    expect(formatSize(25_000_000, false)).toBe("25 MB");
  });
});

describe("paths", () => {
  it("breadcrumbs", () => {
    expect(breadcrumbs("")).toEqual([]);
    expect(breadcrumbs("a/b")).toEqual([
      { name: "a", path: "a" },
      { name: "b", path: "a/b" },
    ]);
  });
  it("parentOf", () => {
    expect(parentOf("a/b/c")).toBe("a/b");
    expect(parentOf("a")).toBe("");
    expect(parentOf("")).toBe("");
  });
  it("baseName は区切り文字の違いを吸収する", () => {
    expect(baseName("/Users/x/docs/")).toBe("docs");
    expect(baseName("C:\\Users\\x\\docs")).toBe("docs");
  });
  it("extensionOf", () => {
    expect(extensionOf("report.pdf")).toBe("PDF");
    expect(extensionOf(".gitignore")).toBe("");
    expect(extensionOf("README")).toBe("");
  });
});
