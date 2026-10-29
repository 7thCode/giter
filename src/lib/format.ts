import type { FileStatus } from "./api";

const dateTime = new Intl.DateTimeFormat("ja-JP", { dateStyle: "medium", timeStyle: "short" });

export function formatTime(unixSeconds: number): string {
  return unixSeconds > 0 ? dateTime.format(new Date(unixSeconds * 1000)) : "--";
}

export function formatSize(bytes: number, isDir: boolean): string {
  if (isDir) return "--";
  if (bytes < 1000) return `${bytes} バイト`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1000;
  let i = 0;
  while (value >= 1000 && i < units.length - 1) {
    value /= 1000;
    i++;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[i]}`;
}

export const statusLabel: Record<FileStatus, string> = {
  saved: "",
  new: "新規",
  modified: "変更あり",
};

export function baseName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

/** `a/b/c` → [{name:"a",path:"a"},{name:"b",path:"a/b"},{name:"c",path:"a/b/c"}] */
export function breadcrumbs(rel: string): { name: string; path: string }[] {
  const parts = rel.split("/").filter(Boolean);
  return parts.map((name, i) => ({ name, path: parts.slice(0, i + 1).join("/") }));
}

export function parentOf(rel: string): string {
  return rel.split("/").filter(Boolean).slice(0, -1).join("/");
}

export function extensionOf(name: string): string {
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(i + 1, i + 5).toUpperCase() : "";
}

/** 案ごとの色。本案はアクセント色、別案は名前から決めた色(同じ案はいつも同じ色)。 */
export function planColor(id: string, isPrimary: boolean): string {
  if (isPrimary) return "var(--accent)";
  let hash = 0;
  for (const ch of id) hash = (hash * 31 + ch.codePointAt(0)!) >>> 0;
  // 青紫(本案)と紛らわしくならないよう、色相を 20〜200 度の暖色〜緑系に寄せる
  return `hsl(${20 + (hash % 180)} 70% 42%)`;
}
