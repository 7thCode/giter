<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { ask, open } from "@tauri-apps/plugin-dialog";
  import { startDrag } from "@crabnebula/tauri-plugin-drag";
  import { api, errorMessage, type Entry, type LargeFile, type Plan, type Version } from "$lib/api";
  import { baseName, breadcrumbs, formatSize, formatTime, parentOf, planColor } from "$lib/format";
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
  /** 保存前に警告する大きいファイル。空でなければ確認画面を出す */
  let largeFiles = $state<LargeFile[]>([]);
  let historyBytes = $state(0);

  let plans = $state<Plan[]>([]);
  let planMenuOpen = $state(false);
  let createOpen = $state(false);
  let newPlanName = $state("");
  let createError = $state("");

  const currentPlan = $derived(plans.find((p) => p.is_current) ?? null);
  const primaryPlan = $derived(plans.find((p) => p.is_primary) ?? null);

  /** 履歴がこれを超えたら注意を促す */
  const HISTORY_WARN_BYTES = 1_000_000_000;

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

  /** いま行われているドラッグが、一覧内のファイルを動かしているだけか(外部からの追加ではない) */
  let ownDrag = false;

  function isInsideRoot(path: string): boolean {
    if (!root) return false;
    const sep = root.includes("\\") ? "\\" : "/";
    return path === root || path.startsWith(root.endsWith(sep) ? root : root + sep);
  }

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
      say("戻しました。内容を確認して「保存」で記録できます");
      refresh();
      void loadSize(root);
    } catch (e) {
      say(errorMessage(e));
    }
  }

  async function loadSize(r: string) {
    try {
      const bytes = await api.historySize(r);
      if (r === root) historyBytes = bytes;
    } catch {
      // サイズ表示ができなくても操作は続けられる
    }
  }

  $effect(() => {
    const r = root;
    if (r) untrack(() => void loadSize(r));
  });

  async function loadPlans(r: string) {
    try {
      const list = await api.listPlans(r);
      if (r === root) plans = list;
    } catch (e) {
      say(errorMessage(e));
    }
  }

  $effect(() => {
    const r = root;
    if (r) untrack(() => void loadPlans(r));
    else plans = [];
  });

  /** 案を作る/切り替えたあとの更新。フォルダの中身が入れ替わるので、表示位置と選択は最初に戻す */
  async function afterPlanChange(message: string) {
    if (!root) return;
    selected = null;
    cwd = "";
    await loadPlans(root);
    refresh();
    void loadSize(root);
    say(message);
  }

  /** 別の案に切り替える。フォルダの中身が入れ替わるので、未保存の変更は先に保存してもらう */
  async function switchTo(plan: Plan) {
    planMenuOpen = false;
    if (!root || plan.is_current) return;
    try {
      if ((await api.pendingCount(root)) > 0) {
        const from = currentPlan?.name ?? "いまの案";
        const ok = await ask(
          `未保存の変更があります。「${plan.name}」に切り替えると、フォルダの中身が入れ替わります。\n今の変更を「${from}」に保存してから切り替えますか?`,
          { title: "保存してから切り替えますか?", kind: "warning", okLabel: "保存して切り替える", cancelLabel: "やめる" },
        );
        if (!ok) return;
        if ((await api.pendingLarge(root)).length > 0) {
          saveOpen = true;
          say("大きいファイルがあります。先に「保存」で扱いを選んでください");
          return;
        }
        await api.saveNow(root, "切り替え前の保存");
      }
      await api.switchPlan(root, plan.id);
      await afterPlanChange(`「${plan.name}」に切り替えました。開いているファイルは、開き直してください`);
    } catch (e) {
      say(errorMessage(e));
    }
  }

  function openCreate() {
    planMenuOpen = false;
    newPlanName = "";
    createError = "";
    createOpen = true;
  }

  async function submitCreate(event: SubmitEvent) {
    event.preventDefault();
    if (!root) return;
    const name = newPlanName.trim();
    try {
      await api.createPlan(root, name);
      createOpen = false;
      await afterPlanChange(`別案「${name}」を作りました。ここでの変更は本案に影響しません`);
    } catch (e) {
      createError = errorMessage(e);
    }
  }

  function closeSave() {
    saveOpen = false;
    largeFiles = [];
  }

  /** 送信時、大きいファイルがあれば保存せずに確認画面を出す */
  async function submitSave(event: SubmitEvent) {
    event.preventDefault();
    if (!root) return;
    try {
      const large = await api.pendingLarge(root);
      if (large.length > 0) {
        largeFiles = large;
        return;
      }
      await save(false);
    } catch (e) {
      say(errorMessage(e));
    }
  }

  /** `excludeNew` なら、大きい新規ファイルを以後の保存から外してから保存する */
  async function save(excludeNew: boolean) {
    if (!root) return;
    try {
      if (excludeNew) {
        await api.excludePaths(
          root,
          largeFiles.filter((f) => f.is_new).map((f) => f.path),
        );
      }
      const saved = await api.saveNow(root, saveMessage);
      say(saved ? "保存しました" : "保存する変更はありません");
      saveMessage = "";
      closeSave();
      refresh();
      void loadSize(root);
    } catch (e) {
      say(errorMessage(e));
    }
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      planMenuOpen = false;
      createOpen = false;
    }
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
      event.preventDefault();
      if (root) saveOpen = true;
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
        // 一覧内でファイルをドラッグしただけ(管理フォルダ内のファイル)なら、追加の表示は出さない
        if (p.type === "enter") ownDrag = p.paths.length > 0 && p.paths.every(isInsideRoot);
        if (p.type === "enter" || p.type === "over") dropping = root !== null && !ownDrag;
        else if (p.type === "leave") dropping = false;
        else if (p.type === "drop") {
          dropping = false;
          ownDrag = false;
          if (!root) return;
          try {
            // 管理フォルダ内のファイルなどは、Rust 側で無視される(コピーされない)
            const added = await api.addFiles(root, cwd, p.paths);
            if (added.length > 0) say(`${added.length} 件を追加しました`);
            refresh();
          } catch (e) {
            say(errorMessage(e));
          }
        }
      }),
    ];
    // 未保存の変更があるまま閉じようとしたら確認する(ハンドラの完了までウィンドウは閉じない)
    unlisten.push(
      getCurrentWindow().onCloseRequested(async (event) => {
        if (!root) return;
        try {
          if ((await api.pendingCount(root)) === 0) return;
          const saveAndClose = await ask("未保存の変更があります。保存してから閉じますか?", {
            title: "終了する前に",
            kind: "warning",
            okLabel: "保存して閉じる",
            cancelLabel: "閉じない",
          });
          if (!saveAndClose) return event.preventDefault();
          // 大きいファイルは黙って保存せず、保存画面で選んでもらう
          if ((await api.pendingLarge(root)).length > 0) {
            event.preventDefault();
            saveOpen = true;
            say("大きいファイルがあります。保存の方法を選んでください");
            return;
          }
          await api.saveNow(root, "終了時の保存");
        } catch (e) {
          // 確認できなかったときは閉じずに状況を見せる
          event.preventDefault();
          say(errorMessage(e));
        }
      }),
    );
    return () => unlisten.forEach((u) => u.then((f) => f()));
  });
</script>

<svelte:window onkeydown={onKeydown} />

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
        {#if dir === root && plans.length > 1}
          <li class="plans-wrap">
            <ul class="plans" aria-label="案">
              {#each plans as plan (plan.id)}
                <li>
                  <button type="button" class="plan-item" class:current={plan.is_current} aria-current={plan.is_current} onclick={() => switchTo(plan)}>
                    <i class="dot" style:--plan={planColor(plan.id, plan.is_primary)}></i>
                    <span>{plan.name}</span>
                  </button>
                </li>
              {/each}
            </ul>
          </li>
        {/if}
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
        <div class="plan">
          <button
            type="button"
            class="tool plan-pill"
            aria-haspopup="menu"
            aria-expanded={planMenuOpen}
            title="案を切り替える・別案を作る"
            style:--plan={currentPlan ? planColor(currentPlan.id, currentPlan.is_primary) : "var(--accent)"}
            onclick={() => (planMenuOpen = !planMenuOpen)}
          >
            <i class="dot"></i>案: {currentPlan?.name ?? "本案"} ▾
          </button>
          {#if planMenuOpen}
            <div class="menu" role="menu">
              {#each plans as plan (plan.id)}
                <button type="button" role="menuitemradio" aria-checked={plan.is_current} class="menu-item" onclick={() => switchTo(plan)}>
                  <i class="dot" style:--plan={planColor(plan.id, plan.is_primary)}></i>
                  <span class="menu-name">{plan.name}</span>
                  {#if plan.is_current}<span aria-hidden="true">✓</span>{/if}
                </button>
              {/each}
              <hr />
              <button type="button" role="menuitem" class="menu-item" onclick={openCreate}>＋ 別案を作る…</button>
            </div>
          {/if}
        </div>
        <div class="seg" role="group" aria-label="表示の切り替え">
          <button type="button" class:on={view === "list"} aria-pressed={view === "list"} onclick={() => setView("list")}>リスト</button>
          <button type="button" class:on={view === "icon"} aria-pressed={view === "icon"} onclick={() => setView("icon")}>アイコン</button>
        </div>
        <div class="save">
          <button type="button" class="tool primary" aria-expanded={saveOpen} onclick={() => (saveOpen ? closeSave() : (saveOpen = true))}>
            保存{#if pending > 0}<span class="count">{pending}</span>{/if}
          </button>
          {#if saveOpen}
            {#if largeFiles.length > 0}
              <div class="popover" role="alertdialog" aria-labelledby="large-title">
                <strong id="large-title">大きいファイルがあります</strong>
                <p class="note">
                  保存すると、変更のたびにファイル全体が履歴に増え、ディスクを使います。
                  また、100 MB を超えるファイルは GitHub へ送れません(今後の同期機能)。
                </p>
                <ul class="large-list">
                  {#each largeFiles as file (file.path)}
                    <li>
                      <span class="path">{file.path}</span>
                      <span class="muted">{formatSize(file.size, false)}{file.is_new ? "" : "(保存済み)"}</span>
                    </li>
                  {/each}
                </ul>
                {#if largeFiles.some((f) => !f.is_new)}
                  <p class="note">「保存済み」のファイルは、除外できません。</p>
                {/if}
                <div class="actions wrap">
                  <button type="button" onclick={closeSave}>キャンセル</button>
                  <button type="button" onclick={() => save(false)}>そのまま保存</button>
                  {#if largeFiles.some((f) => f.is_new)}
                    <button type="button" class="primary" onclick={() => save(true)}>除外して保存</button>
                  {/if}
                </div>
              </div>
            {:else}
              <form class="popover" onsubmit={submitSave}>
                <label for="memo">メモ(省略できます)</label>
                <!-- svelte-ignore a11y_autofocus -->
                <input id="memo" bind:value={saveMessage} placeholder="例: 見積書を修正" autofocus />
                <div class="actions">
                  <button type="button" onclick={closeSave}>キャンセル</button>
                  <button type="submit" class="primary">保存</button>
                </div>
              </form>
            {/if}
          {/if}
        </div>
        <button type="button" class="tool" aria-pressed={showHistory} onclick={() => (showHistory = !showHistory)}>履歴</button>
      </header>

      {#if currentPlan && !currentPlan.is_primary}
        <div class="banner" role="status" style:--plan={planColor(currentPlan.id, false)}>
          <span>別案「<strong>{currentPlan.name}</strong>」で作業中です。本案には影響しません。</span>
          {#if primaryPlan}<button type="button" onclick={() => switchTo(primaryPlan)}>本案に戻る</button>{/if}
        </div>
      {/if}

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
        <span>{pending > 0 ? `未保存の変更 ${pending} 件(⌘S で保存)` : "すべて保存済み"}</span>
        <span
          class="history-size"
          class:warn={historyBytes > HISTORY_WARN_BYTES}
          title="履歴がディスクで使っている量です。大きいファイルは、変更のたびに丸ごと増えます"
        >
          履歴 {formatSize(historyBytes, false)}
        </span>
      </footer>
    {:else}
      <div class="welcome">
        <h1>フォルダの変更を、かんたんに記録</h1>
        <p>フォルダを選んで、区切りのよいところで「保存」を押すと、いつでもその時点に戻せます。</p>
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

{#if createOpen}
  <button type="button" class="backdrop" aria-label="閉じる" tabindex="-1" onclick={() => (createOpen = false)}></button>
  <div class="modal" role="dialog" aria-modal="true" aria-labelledby="create-title">
    <form onsubmit={submitCreate}>
    <h3 id="create-title">別案を作る</h3>
    <p class="note">今の状態から、もうひとつの案を始めます。本案は変わりません。いつでも本案に戻れます。</p>
    {#if pending > 0}
      <p class="note strong">いまの未保存の変更は、この別案に含まれます。</p>
    {/if}
    <label for="plan-name">別案の名前</label>
    <!-- svelte-ignore a11y_autofocus -->
    <input id="plan-name" bind:value={newPlanName} placeholder="例: 価格を上げる案" autofocus />
    {#if createError}<p class="error" role="alert">{createError}</p>{/if}
    <div class="actions">
      <button type="button" onclick={() => (createOpen = false)}>キャンセル</button>
      <button type="submit" class="primary" disabled={newPlanName.trim() === ""}>作る</button>
    </div>
    </form>
  </div>
{/if}

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
  .dot {
    display: inline-block;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--plan, var(--accent));
    flex: none;
  }
  .plans-wrap {
    display: block;
  }
  .plans {
    list-style: none;
    margin: 0 0 4px 22px;
    padding: 0;
  }
  .plan-item {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 3px 6px;
    background: none;
    border: 0;
    border-radius: 6px;
    color: var(--text);
    text-align: left;
    cursor: pointer;
    font-size: 13px;
  }
  .plan-item.current {
    font-weight: 600;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .plan {
    position: relative;
  }
  .plan-pill {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    border-color: color-mix(in srgb, var(--plan) 55%, var(--border));
  }
  .menu {
    position: absolute;
    right: 0;
    top: calc(100% + 6px);
    z-index: 10;
    min-width: 220px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 6px;
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.2);
  }
  .menu hr {
    border: 0;
    border-top: 1px solid var(--border);
    margin: 4px 0;
  }
  .menu-item {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    background: none;
    border: 0;
    border-radius: 6px;
    color: var(--text);
    text-align: left;
    cursor: pointer;
  }
  .menu-item:hover {
    background: var(--button-hover);
  }
  .menu-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 6px 14px;
    font-size: 13px;
    color: var(--text);
    background: color-mix(in srgb, var(--plan) 16%, var(--bg));
    border-bottom: 2px solid var(--plan);
  }
  .banner button {
    background: var(--button);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 2px 12px;
    color: var(--text);
    cursor: pointer;
    white-space: nowrap;
  }
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 20;
    background: rgb(0 0 0 / 0.35);
    border: 0;
    padding: 0;
    cursor: default;
  }
  .modal {
    position: fixed;
    z-index: 21;
    left: 50%;
    top: 22%;
    transform: translateX(-50%);
    width: min(420px, calc(100vw - 32px));
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 18px;
    box-shadow: 0 16px 48px rgb(0 0 0 / 0.35);
  }
  .modal form {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .modal h3 {
    margin: 0;
    font-size: 16px;
  }
  .modal label {
    font-size: 12px;
    color: var(--text-dim);
  }
  .modal input {
    font: inherit;
    padding: 6px 9px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
  }
  .modal .actions button {
    background: var(--button);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 4px 14px;
    color: var(--text);
    cursor: pointer;
  }
  .modal .actions button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }
  .modal .actions button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .note.strong {
    color: var(--text);
    font-weight: 600;
  }
  .error {
    margin: 0;
    font-size: 12px;
    color: #d92d20;
  }
  .actions.wrap {
    flex-wrap: wrap;
  }
  .note {
    margin: 0;
    font-size: 12px;
    color: var(--text-dim);
  }
  .large-list {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 140px;
    overflow-y: auto;
    font-size: 12px;
  }
  .large-list li {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    padding: 3px 0;
  }
  .large-list .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .muted {
    color: var(--text-dim);
    white-space: nowrap;
  }
  .history-size.warn {
    background: var(--mod-bg);
    color: var(--mod-fg);
    border-radius: 9px;
    padding: 0 8px;
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
