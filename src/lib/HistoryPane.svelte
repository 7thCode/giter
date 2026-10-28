<script lang="ts">
  import type { Version } from "./api";
  import { formatTime } from "./format";

  let {
    versions,
    target,
    canExport,
    onrestore,
    onprepare,
    ondragout,
  }: {
    versions: Version[];
    /** 選択中の項目名。未選択ならフォルダ全体 */
    target: string | null;
    /** 選択中がファイルなら、過去バージョンを Finder にドラッグして取り出せる */
    canExport: boolean;
    onrestore: (version: Version) => void;
    /** ドラッグ開始前(ボタンを押した時点)に、取り出すファイルを用意しておく */
    onprepare: (version: Version) => void;
    ondragout: (version: Version) => void;
  } = $props();
</script>

<aside class="pane" aria-label="履歴">
  <h2>履歴</h2>
  <p class="scope">{target ? `「${target}」の変更` : "フォルダ全体の変更"}</p>
  {#if versions.length === 0}
    <p class="none">まだ履歴がありません</p>
  {:else}
    <ol>
      {#each versions as version, i (version.id)}
        <li
          draggable={canExport}
          onpointerdown={() => canExport && onprepare(version)}
          ondragstart={(e) => {
            e.preventDefault();
            if (canExport) ondragout(version);
          }}
        >
          <div class="when">
            {formatTime(version.time)}
            {#if i === 0}<span class="now">最新</span>{/if}
          </div>
          <div class="msg">{version.message}</div>
          {#if i > 0}
            <button type="button" onclick={() => onrestore(version)}>この時点に戻す</button>
          {/if}
        </li>
      {/each}
    </ol>
    {#if canExport}<p class="tip">過去の版は Finder へドラッグして取り出せます</p>{/if}
  {/if}
</aside>

<style>
  .pane {
    width: 260px;
    flex: none;
    border-left: 1px solid var(--border);
    background: var(--sidebar);
    overflow-y: auto;
    padding: 12px;
  }
  h2 {
    margin: 0;
    font-size: 13px;
  }
  .scope,
  .none,
  .tip {
    margin: 4px 0 10px;
    font-size: 12px;
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
  ol {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  li {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 8px 10px;
    font-size: 12px;
  }
  li[draggable="true"] {
    cursor: grab;
  }
  .when {
    color: var(--text-dim);
  }
  .now {
    margin-left: 6px;
    color: var(--accent);
  }
  .msg {
    margin: 2px 0 6px;
    font-size: 13px;
    overflow-wrap: anywhere;
  }
  button {
    font: inherit;
    font-size: 12px;
    padding: 2px 10px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--button);
    color: var(--text);
    cursor: pointer;
  }
  button:hover {
    background: var(--button-hover);
  }
</style>
