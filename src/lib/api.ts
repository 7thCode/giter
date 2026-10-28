import { invoke } from "@tauri-apps/api/core";

export type FileStatus = "saved" | "new" | "modified";

export interface Entry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  /** UNIX 秒 */
  modified: number;
  status: FileStatus;
}

export interface Version {
  id: string;
  message: string;
  /** UNIX 秒 */
  time: number;
}

export const api = {
  openFolder: (root: string) => invoke<void>("open_folder", { root }),
  listDir: (root: string, rel: string) => invoke<Entry[]>("list_dir", { root, rel }),
  pendingCount: (root: string) => invoke<number>("pending_count", { root }),
  saveNow: (root: string, message: string) => invoke<boolean>("save_now", { root, message }),
  history: (root: string, rel: string | null) => invoke<Version[]>("history", { root, rel }),
  restore: (root: string, id: string, rel: string | null) => invoke<void>("restore", { root, id, rel }),
  addFiles: (root: string, dest: string, paths: string[]) =>
    invoke<string[]>("add_files", { root, dest, paths }),
  exportVersion: (root: string, id: string, rel: string) =>
    invoke<string>("export_version", { root, id, rel }),
  openItem: (root: string, rel: string) => invoke<void>("open_item", { root, rel }),
  dragIcon: () => invoke<string>("drag_icon"),
};

/** Tauri のエラーは文字列で届く。 */
export function errorMessage(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}
