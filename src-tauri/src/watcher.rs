//! フォルダの変更を監視し、画面に更新を通知する。保存はユーザーの操作でのみ行う。

use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher as _};
use tauri::{AppHandle, Emitter};

/// 変更が続いている間は通知をまとめ、この時間静かになったらもう一度通知する。
const QUIET_PERIOD: Duration = Duration::from_millis(500);

/// 破棄すると監視が止まる(通知元の `RecommendedWatcher` が落ち、スレッドが終了する)。
pub struct Watcher {
    _inner: RecommendedWatcher,
}

fn touches_worktree(root: &Path, event: &Event) -> bool {
    event.paths.iter().any(|p| {
        let rel = p.strip_prefix(root).unwrap_or(p);
        !rel.components().any(|c| matches!(c, Component::Normal(n) if n == ".git"))
    })
}

pub fn start(app: AppHandle, root: PathBuf) -> notify::Result<Watcher> {
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut inner = notify::recommended_watcher(tx)?;
    inner.watch(&root, RecursiveMode::Recursive)?;

    thread::spawn(move || {
        while let Ok(first) = rx.recv() {
            if !first.is_ok_and(|e| touches_worktree(&root, &e)) {
                continue;
            }
            let _ = app.emit("repo-changed", ());
            // 変更が続いている間は待ち、落ち着いたら最終状態をもう一度通知する
            while rx.recv_timeout(QUIET_PERIOD).is_ok() {}
            let _ = app.emit("repo-changed", ());
        }
    });

    Ok(Watcher { _inner: inner })
}
