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

export interface LargeFile {
  path: string;
  size: number;
  /** まだ一度も保存していない(除外できるのはこれだけ) */
  is_new: boolean;
}

/** 案(Git のブランチ)。`id` は内部名、`name` は画面に出す名前。 */
export interface Plan {
  id: string;
  name: string;
  is_current: boolean;
  /** 本案(最初の案) */
  is_primary: boolean;
  /** しまってある案 */
  is_archived: boolean;
  /** 最後に保存した時刻(UNIX 秒) */
  time: number;
}

export interface PlanChange {
  path: string;
  kind: "added" | "modified" | "deleted";
  /** 本案でも別案でも、同じファイルが別々に変更されている */
  conflict: boolean;
}

/** 競合したファイルの扱い: 本案の版 / 別案の版 / 両方残す */
export type Resolution = "primary" | "plan" | "both";

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
  listPlans: (root: string) => invoke<Plan[]>("list_plans", { root }),
  createPlan: (root: string, name: string) => invoke<string>("create_plan", { root, name }),
  switchPlan: (root: string, id: string) => invoke<void>("switch_plan", { root, id }),
  archivePlan: (root: string, id: string) => invoke<void>("archive_plan", { root, id }),
  reopenPlan: (root: string, id: string) => invoke<void>("reopen_plan", { root, id }),
  planChanges: (root: string, id: string) => invoke<PlanChange[]>("plan_changes", { root, id }),
  adoptPlan: (root: string, id: string, resolutions: Record<string, Resolution>) =>
    invoke<void>("adopt_plan", { root, id, resolutions }),
  pendingLarge: (root: string) => invoke<LargeFile[]>("pending_large", { root }),
  excludePaths: (root: string, paths: string[]) => invoke<void>("exclude_paths", { root, paths }),
  historySize: (root: string) => invoke<number>("history_size", { root }),
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
