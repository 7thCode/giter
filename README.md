# giter

Git を使ったことがない人のための、シンプルなバージョン管理デスクトップアプリ。
フォルダの変更を「保存」して、いつでも以前の状態に「戻す」ことができます。
Tauri 2 + Svelte 5(SvelteKit)製。ローカルだけで動作します。

ダウンロード: [GitHub Pages](https://7thcode.github.io/giter/) / [Releases](https://github.com/7thCode/giter/releases)

## できること

- Finder のようなフォルダ表示(リスト / アイコン)。変更のあるファイルに「新規」「変更あり」を表示
- ファイルをウィンドウへドロップして追加(元のファイルは残し、コピーする)。管理フォルダ内のファイルをウィンドウ内でドラッグ&ドロップしても、コピーはしない
- ファイルや過去の版を Finder などへドラッグして取り出し
- 「保存」ボタンまたは `Cmd/Ctrl+S` で、メモ付きで保存(自動保存はしない)。未保存のまま閉じると確認を出す
- 履歴から、ファイル / フォルダを以前の状態に戻す
  - 戻した結果は保存しない(履歴に「戻した」という項目は作らない)。「変更あり」になるので、残したければ保存する
  - 戻す前に未保存の変更があれば、失わないよう自動で1件保存する

- 別案(ブランチ)を試す。「案」メニューから別案を作ると、本案はそのまま残る
  - 別案の間は色付きの帯を表示。切り替えるとフォルダの中身がその案の状態に入れ替わる(未保存の変更があれば先に保存)
  - 「本案に採用」で、変わるファイルを一覧で確認してから反映。本案と別案の両方で変えたファイルは、ファイルごとに「本案の版 / 別案の版 / 両方残す」を選ぶ(行単位のマージはしない)
  - 不要な案は「ごみ箱に入れる」。ごみ箱から「元に戻す」ができ、「ごみ箱を空にする」まで保存は残る(空にしても履歴のサイズはすぐには減らない)。入れる前に、本案に採用済みか(失うものがないか)を表示

用語は Git と対応させています: 保存 = commit、戻す = checkout、案 = branch、採用 = merge。操作はすべて、アプリに同梱の libgit2 で行うため、`git` コマンドのインストールは不要です。

## 開発

必要なもの: Node.js 22 以上、Rust(stable)、[Tauri の前提環境](https://v2.tauri.app/start/prerequisites/)

```bash
npm ci
npm run tauri dev      # 開発起動
npm run check          # 型チェック
npm test               # フロントエンドのテスト(vitest)
cargo test --manifest-path src-tauri/Cargo.toml   # Git 操作のテスト
```

### 構成

| 場所 | 役割 |
|---|---|
| `src-tauri/src/repo.rs` | 保存・履歴・復元・ファイル追加(git2)。テスト付き |
| `src-tauri/src/watcher.rs` | フォルダの変更を検知して画面へ通知(保存はしない) |
| `src-tauri/src/lib.rs` | Tauri コマンド |
| `src/routes/+page.svelte` | メイン画面(ツールバー、サイドバー、ドロップ処理) |
| `src/lib/` | `FileList` / `HistoryPane` / API ラッパー / 整形関数 |

## リリース

```bash
git tag v0.1.0 && git push origin v0.1.0
```

タグの push で [actions-templates](https://github.com/7thCode/actions-templates) の
`tauri-multiplatform-release.yml` が動き、macOS(universal `.dmg`)/ Windows(NSIS `.exe`)/
Linux(`.AppImage` / `.deb`)のインストーラーが GitHub Release に登録されます。
リリースのたびに `src-tauri/tauri.conf.json` の `version` と、`docs/index.html` のバージョン・リンクを更新してください。
インストーラーは未署名です。

## ブランチ運用

作業はフィーチャーブランチで行い、`main` へは Pull Request 経由でマージします(直接 push は不可)。
