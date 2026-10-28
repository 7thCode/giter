<script lang="ts">
  import type { Entry } from "./api";
  import FileIcon from "./FileIcon.svelte";
  import { formatSize, formatTime, statusLabel } from "./format";

  let {
    entries,
    view,
    selected = $bindable(null),
    onopen,
    ondragout,
  }: {
    entries: Entry[];
    view: "list" | "icon";
    selected: string | null;
    onopen: (entry: Entry) => void;
    ondragout: (entry: Entry) => void;
  } = $props();

  function keydown(e: KeyboardEvent, entry: Entry) {
    if (e.key === "Enter") onopen(entry);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const i = entries.findIndex((x) => x.path === entry.path);
      const next = entries[i + (e.key === "ArrowDown" ? 1 : -1)];
      if (next) {
        selected = next.path;
        (e.currentTarget as HTMLElement).parentElement
          ?.querySelector<HTMLElement>(`[data-path="${CSS.escape(next.path)}"]`)
          ?.focus();
      }
    }
  }

  function dragstart(e: DragEvent, entry: Entry) {
    e.preventDefault();
    selected = entry.path;
    ondragout(entry);
  }
</script>

{#if entries.length === 0}
  <div class="empty">
    <p>このフォルダは空です</p>
    <p class="hint">ファイルをここにドロップすると追加されます</p>
  </div>
{:else if view === "list"}
  <div class="table" role="listbox" aria-label="ファイル一覧">
    <div class="head" aria-hidden="true">
      <span class="name">名前</span><span class="status">状態</span><span class="date">更新日</span><span class="size">サイズ</span>
    </div>
    {#each entries as entry (entry.path)}
      <div
        class="row"
        class:selected={selected === entry.path}
        role="option"
        aria-selected={selected === entry.path}
        tabindex="0"
        draggable="true"
        data-path={entry.path}
        onclick={() => (selected = entry.path)}
        ondblclick={() => onopen(entry)}
        onkeydown={(e) => keydown(e, entry)}
        ondragstart={(e) => dragstart(e, entry)}
      >
        <span class="name"><FileIcon name={entry.name} isDir={entry.is_dir} /> <span class="label">{entry.name}</span></span>
        <span class="status">
          {#if entry.status !== "saved"}<span class="badge {entry.status}">{statusLabel[entry.status]}</span>{/if}
        </span>
        <span class="date">{formatTime(entry.modified)}</span>
        <span class="size">{formatSize(entry.size, entry.is_dir)}</span>
      </div>
    {/each}
  </div>
{:else}
  <div class="grid" role="listbox" aria-label="ファイル一覧">
    {#each entries as entry (entry.path)}
      <div
        class="cell"
        class:selected={selected === entry.path}
        role="option"
        aria-selected={selected === entry.path}
        tabindex="0"
        draggable="true"
        data-path={entry.path}
        onclick={() => (selected = entry.path)}
        ondblclick={() => onopen(entry)}
        onkeydown={(e) => keydown(e, entry)}
        ondragstart={(e) => dragstart(e, entry)}
      >
        <span class="icon"><FileIcon name={entry.name} isDir={entry.is_dir} size={56} /></span>
        <span class="label">{entry.name}</span>
        {#if entry.status !== "saved"}<span class="badge {entry.status}">{statusLabel[entry.status]}</span>{/if}
      </div>
    {/each}
  </div>
{/if}

<style>
  .empty {
    height: 100%;
    display: grid;
    place-content: center;
    text-align: center;
    color: var(--text-dim);
  }
  .hint {
    font-size: 12px;
  }
  .table {
    display: flex;
    flex-direction: column;
    font-size: 13px;
  }
  .head,
  .row {
    display: grid;
    grid-template-columns: minmax(180px, 1fr) 90px 150px 90px;
    align-items: center;
    padding: 0 12px;
    gap: 8px;
  }
  .head {
    position: sticky;
    top: 0;
    background: var(--bg);
    height: 26px;
    color: var(--text-dim);
    font-size: 12px;
    border-bottom: 1px solid var(--border);
  }
  .row {
    height: 28px;
    border-radius: 6px;
    margin: 0 6px;
    padding: 0 6px;
    cursor: default;
    user-select: none;
  }
  .row:nth-child(even) {
    background: var(--stripe);
  }
  .row.selected,
  .cell.selected {
    background: var(--accent);
    color: var(--on-accent);
  }
  .row:focus-visible,
  .cell:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .name {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .date,
  .size {
    color: var(--text-dim);
    white-space: nowrap;
  }
  .selected .date,
  .selected .size {
    color: inherit;
  }
  .size {
    text-align: right;
  }
  .badge {
    font-size: 11px;
    padding: 1px 7px;
    border-radius: 9px;
    white-space: nowrap;
  }
  .badge.new {
    background: var(--new-bg);
    color: var(--new-fg);
  }
  .badge.modified {
    background: var(--mod-bg);
    color: var(--mod-fg);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(104px, 1fr));
    gap: 6px;
    padding: 12px;
  }
  .cell {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 8px 4px;
    border-radius: 8px;
    font-size: 12px;
    text-align: center;
    cursor: default;
    user-select: none;
  }
  .cell .label {
    max-width: 100%;
    white-space: normal;
    word-break: break-all;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
</style>
