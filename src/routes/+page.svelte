<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { ask, open } from "@tauri-apps/plugin-dialog";
  import { startDrag } from "@crabnebula/tauri-plugin-drag";
  import { api, errorMessage, type Entry, type Version } from "$lib/api";
  import { baseName, breadcrumbs, formatTime, parentOf } from "$lib/format";
  import FileList from "$lib/FileList.svelte";
  import HistoryPane from "$lib/HistoryPane.svelte";

  let roots = $state<string[]>([]);
  let root = $state<string | null>(null);
  let cwd = $state("");
  let entries = $state<Entry[]>([]);
  let selected = $state<string | null>(null);
  let view = $state<"list" | "icon">("list");
  let showHistory = $state(true);
  let versions = $state<Version[]>([]);
  let pending = $state(0);
  let dropping = $state(false);
  let notice = $state("");
  let saveOpen = $state(false);
  let saveMessage = $state("");

  const selectedEntry = $derived(entries.find((e) => e.path === selected) ?? null);
  /** 履歴・復元の対象。選択があればそれ、なければ表示中のフォルダ(ルートなら全体) */
  const scopeRel = $derived(selected ?? (cwd || null));
  const scopeName = $derived(selectedEntry?.name ?? (cwd ? baseName(cwd) : null));
  const crumbs = $derived(breadcrumbs(cwd));

  let noticeTimer: ReturnType<typeof setTimeout> | undefined;
  function say(message: string) {
    notice = message;
    clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => (notice = ""), 4000);
  }

  function remember() {
    try {
      localStorage.setItem("giter.roots", JSON.stringify(roots));
      localStorage.setItem("giter.root", root ?? "");
      localStorage.setItem("giter.view", view);
    } catch {
      // 保存できなくてもアプリは動く
    }
  }

  // 古い応答で新しい表示を上書きしないための番号
  let listSeq = 0;
  let historySeq = 0;

  async function loadList(r: string, c: string) {
    const seq = ++listSeq;
    try {
      const [list, count] = await Promise.all([api.listDir(r, c), api.pendingCount(r)]);
      if (seq !== listSeq) return;
      entries = list;
      pending = count;
      if (selected && !list.some((e) => e.path === selected)) selected = null;
    } catch (e) {
      say(errorMessage(e));
    }
  }

  async function loadHistory(r: string, rel: string | null) {
    const seq = ++historySeq;
    try {
      const list = await api.history(r, rel);
      if (seq === historySeq) versions = list;
    } catch (e) {
      say(errorMessage(e));
    }
  }

  function refresh() {
    if (!root) return;
    void loadList(root, cwd);
    void loadHistory(root, scopeRel);
  }

  $effect(() => {
    const r = root;
    const c = cwd;
    if (r) untrack(() => void loadList(r, c));
  });

  $effect(() => {
    const r = root;
    const rel = scopeRel;
    if (r) untrack(() => void loadHistory(r, rel));
  });

  async function openRoot(dir: string) {
    try {
      await api.openFolder(dir);
    } catch (e) {
      say(errorMessage(e));
      return;
    }
    if (!roots.includes(dir)) roots = [...roots, dir];
    root = dir;
    cwd = "";
    selected = null;
    remember();
  }

  async function addFolder() {
    const dir = await open({ directory: true, multiple: false, title: "管理するフォルダを選択" });
    if (typeof dir !== "string") return;
    if (!roots.includes(dir)) {
      const ok = await ask(
        `「${baseName(dir)}」の変更を記録します。\n中のファイルは変更されません。履歴を保存する隠しフォルダ(.git)が作られます。`,
        { title: "このフォルダを管理しますか?", okLabel: "管理する", cancelLabel: "キャンセル" },
      );
      if (!ok) return;
    }
    await openRoot(dir);
  }

  function removeRoot(dir: string) {
    roots = roots.filter((r) => r !== dir);
    if (root === dir) {
      root = null;
      entries = [];
      versions = [];
    }
    remember();
  }

  function navigate(rel: string) {
    cwd = rel;
    selected = null;
  }

  async function openEntry(entry: Entry) {
    if (entry.is_dir) return navigate(entry.path);
    if (!root) return;
    try {
      await api.openItem(root, entry.path);
    } catch (e) {
      say(errorMessage(e));
    }
  }

  function absolute(rel: string): string {
    const sep = root?.includes("\\") ? "\\" : "/";
    return `${root}${sep}${rel.split("/").join(sep)}`;
  }

  // ドラッグ開始の瞬間に非同期待ちを挟まないよう、アイコンは先に用意しておく
  let dragIcon = "";

  async function dragOut(entry: Entry) {
    if (!root || !dragIcon) return;
    try {
      await startDrag({ item: [absolute(entry.path)], icon: dragIcon, mode: "copy" });
    } catch (e) {
      say(errorMessage(e));
    }
  }

  const prepared = new Map<string, Promise<string>>();

  function prepareVersion(version: Version) {
    if (!root || !selectedEntry || selectedEntry.is_dir) return;
    const key = `${version.id}:${selectedEntry.path}`;
    if (!prepared.has(key)) prepared.set(key, api.exportVersion(root, version.id, selectedEntry.path));
  }

  async function dragOutVersion(version: Version) {
    if (!root || !selectedEntry || selectedEntry.is_dir || !dragIcon) return;
    prepareVersion(version);
    try {
      const path = await prepared.get(`${version.id}:${selectedEntry.path}`)!;
      await startDrag({ item: [path], icon: dragIcon, mode: "copy" });
    } catch (e) {
      say(errorMessage(e));
    }
  }

  async function restore(version: Version) {
    if (!root) return;
    const target = scopeName ? `「${scopeName}」` : "フォルダ全体";
    const ok = await ask(
      `${target}を ${formatTime(version.time)} の状態に戻します。\n今の状態も履歴に残るので、あとから元に戻せます。`,
      { title: "この時点に戻しますか?", kind: "warning", okLabel: "戻す", cancelLabel: "キャンセル" },
    );
    if (!ok) return;
    try {
      await api.restore(root, version.id, scopeRel);
      say("戻しました");
      refresh();
    } catch (e) {
      say(errorMessage(e));
    }
  }

  async function saveNow(event: SubmitEvent) {
    event.preventDefault();
    if (!root) return;
    try {
      const saved = await api.saveNow(root, saveMessage);
      say(saved ? "保存しました" : "変更はありません(自動保存済みです)");
      saveOpen = false;
      saveMessage = "";
      refresh();
    } catch (e) {
      say(errorMessage(e));
    }
  }

  function setView(next: "list" | "icon") {
    view = next;
    remember();
  }

  onMount(() => {
    try {
      roots = JSON.parse(localStorage.getItem("giter.roots") ?? "[]");
      const saved = localStorage.getItem("giter.view");
      if (saved === "list" || saved === "icon") view = saved;
      const last = localStorage.getItem("giter.root");
      if (last && roots.includes(last)) void openRoot(last);
    } catch {
      // 保存済みの設定が読めなければ初期状態で始める
    }
    api.dragIcon().then((p) => (dragIcon = p)).catch((e) => say(errorMessage(e)));

    const unlisten: Promise<() => void>[] = [
      listen("repo-changed", refresh),
      getCurrentWebview().onDragDropEvent(async (event) => {
        const p = event.payload;
        if (p.type === "enter" || p.type === "over") dropping = root !== null;
        else if (p.type === "leave") dropping = false;
        else if (p.type === "drop") {
          dropping = false;
          if (!root) return;
          try {
            const added = await api.addFiles(root, cwd, p.paths);
            say(`${added.length} 件を追加しました`);
            refresh();
          } catch (e) {
            say(errorMessage(e));
          }
        }
      }),
    ];
    return () => unlisten.forEach((u) => u.then((f) => f()));
  });
</script>

<div class="app">
  <nav class="sidebar" aria-label="管理中のフォルダ">
    <h2>管理中のフォルダ</h2>
    <ul>
      {#each roots as dir (dir)}
        <li class:active={dir === root}>
          <button type="button" class="root" title={dir} onclick={() => openRoot(dir)}>
            <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true">
              <path d="M2 6.5A2.5 2.5 0 0 1 4.5 4h4.2c.5 0 .9.2 1.2.5L11.4 6H19.5A2.5 2.5 0 0 1 22 8.5v9a2.5 2.5 0 0 1-2.5 2.5h-15A2.5 2.5 0 0 1 2 17.5z" fill="var(--folder)" />
            </svg>
            <span>{baseName(dir)}</span>
          </button>
          <button type="button" class="remove" aria-label="{baseName(dir)} を一覧から外す" title="一覧から外す(ファイルは消えません)" onclick={() => removeRoot(dir)}>×</button>
        </li>
      {/each}
    </ul>
    <button type="button" class="add" onclick={addFolder}>＋ フォルダを追加</button>
  </nav>

  <main class="main">
    {#if root}
      <header class="toolbar">
        <button type="button" class="tool" aria-label="ひとつ上のフォルダへ" disabled={cwd === ""} onclick={() => navigate(parentOf(cwd))}>‹</button>
        <div class="crumbs" aria-label="現在の場所">
          <button type="button" onclick={() => navigate("")}>{baseName(root)}</button>
          {#each crumbs as crumb (crumb.path)}
            <span aria-hidden="true">›</span>
            <button type="button" onclick={() => navigate(crumb.path)}>{crumb.name}</button>
          {/each}
        </div>
        <div class="seg" role="group" aria-label="表示の切り替え">
          <button type="button" class:on={view === "list"} aria-pressed={view === "list"} onclick={() => setView("list")}>リスト</button>
          <button type="button" class:on={view === "icon"} aria-pressed={view === "icon"} onclick={() => setView("icon")}>アイコン</button>
        </div>
        <div class="save">
          <button type="button" class="tool primary" aria-expanded={saveOpen} onclick={() => (saveOpen = !saveOpen)}>
            保存{#if pending > 0}<span class="count">{pending}</span>{/if}
          </button>
          {#if saveOpen}
            <form class="popover" onsubmit={saveNow}>
              <label for="memo">メモ(省略できます)</label>
              <!-- svelte-ignore a11y_autofocus -->
              <input id="memo" bind:value={saveMessage} placeholder="例: 見積書を修正" autofocus />
              <div class="actions">
                <button type="button" onclick={() => (saveOpen = false)}>キャンセル</button>
                <button type="submit" class="primary">保存</button>
              </div>
            </form>
          {/if}
        </div>
        <button type="button" class="tool" aria-pressed={showHistory} onclick={() => (showHistory = !showHistory)}>履歴</button>
      </header>

      <div class="content">
        <FileList {entries} {view} bind:selected onopen={openEntry} ondragout={dragOut} />
        {#if dropping}
          <div class="drop-overlay" aria-hidden="true">
            <span>ここにドロップして「{cwd ? baseName(cwd) : baseName(root)}」に追加</span>
          </div>
        {/if}
      </div>

      <footer class="statusbar">
        <span>{entries.length} 項目</span>
        <span>{pending > 0 ? `未保存の変更 ${pending} 件(まもなく自動保存)` : "すべて保存済み"}</span>
      </footer>
    {:else}
      <div class="welcome">
        <h1>フォルダの変更を、かんたんに記録</h1>
        <p>フォルダを選ぶと、ファイルの変更が自動で保存され、いつでも以前の状態に戻せます。</p>
        <button type="button" class="primary big" onclick={addFolder}>フォルダを選ぶ</button>
      </div>
    {/if}
  </main>

  {#if root && showHistory}
    <HistoryPane
      {versions}
      target={scopeName}
      canExport={selectedEntry !== null && !selectedEntry.is_dir}
      onrestore={restore}
      onprepare={prepareVersion}
      ondragout={dragOutVersion}
    />
  {/if}
</div>

{#if notice}
  <div class="toast" role="status">{notice}</div>
{/if}

<style>
  .app {
    display: flex;
    height: 100vh;
  }
  .sidebar {
    width: 200px;
    flex: none;
    background: var(--sidebar);
    border-right: 1px solid var(--border);
    padding: 12px 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    overflow-y: auto;
  }
  h2 {
    margin: 0 6px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-dim);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: flex;
    align-items: center;
    border-radius: 6px;
  }
  li.active {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .root {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 6px;
    background: none;
    border: 0;
    color: var(--text);
    text-align: left;
    cursor: pointer;
  }
  .root span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .remove {
    visibility: hidden;
    background: none;
    border: 0;
    color: var(--text-dim);
    cursor: pointer;
    padding: 2px 6px;
  }
  li:hover .remove,
  .remove:focus-visible {
    visibility: visible;
  }
  .add {
    margin-top: 4px;
    background: none;
    border: 0;
    color: var(--accent);
    text-align: left;
    padding: 5px 6px;
    cursor: pointer;
  }
  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
  }
  .crumbs {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 4px;
    overflow: hidden;
    white-space: nowrap;
    color: var(--text-dim);
  }
  .crumbs button {
    background: none;
    border: 0;
    padding: 2px 4px;
    color: var(--text);
    cursor: pointer;
    border-radius: 4px;
  }
  .crumbs button:last-of-type {
    font-weight: 600;
  }
  .tool,
  .seg button,
  .popover button {
    background: var(--button);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 3px 12px;
    color: var(--text);
    cursor: pointer;
  }
  .tool:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .tool[aria-pressed="true"] {
    background: var(--button-hover);
  }
  .seg {
    display: flex;
  }
  .seg button {
    border-radius: 0;
    padding: 3px 10px;
  }
  .seg button:first-child {
    border-radius: 6px 0 0 6px;
  }
  .seg button:last-child {
    border-radius: 0 6px 6px 0;
    border-left: 0;
  }
  .seg button.on {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }
  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }
  .count {
    margin-left: 6px;
    background: var(--on-accent);
    color: var(--accent);
    border-radius: 9px;
    padding: 0 6px;
    font-size: 11px;
  }
  .save {
    position: relative;
  }
  .popover {
    position: absolute;
    right: 0;
    top: calc(100% + 6px);
    z-index: 10;
    width: 260px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 12px;
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.2);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .popover label {
    font-size: 12px;
    color: var(--text-dim);
  }
  .popover input {
    font: inherit;
    padding: 5px 8px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .content {
    flex: 1;
    min-height: 0;
    position: relative;
    overflow-y: auto;
  }
  .drop-overlay {
    position: absolute;
    inset: 8px;
    border: 2px dashed var(--accent);
    border-radius: 12px;
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    display: grid;
    place-content: center;
    color: var(--accent);
    font-weight: 600;
    pointer-events: none;
  }
  .statusbar {
    display: flex;
    justify-content: space-between;
    padding: 4px 14px;
    border-top: 1px solid var(--border);
    font-size: 12px;
    color: var(--text-dim);
  }
  .welcome {
    height: 100%;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 8px;
    text-align: center;
    padding: 24px;
  }
  .welcome h1 {
    margin: 0;
    font-size: 22px;
  }
  .welcome p {
    margin: 0 0 12px;
    color: var(--text-dim);
    max-width: 30em;
  }
  .big {
    font-size: 15px;
    padding: 8px 22px;
    border-radius: 8px;
    border: 1px solid var(--accent);
    cursor: pointer;
  }
  .toast {
    position: fixed;
    left: 50%;
    bottom: 36px;
    transform: translateX(-50%);
    background: #2c2c2e;
    color: #fff;
    padding: 8px 16px;
    border-radius: 9px;
    font-size: 13px;
    box-shadow: 0 6px 20px rgb(0 0 0 / 0.3);
  }
</style>
