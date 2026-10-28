//! フォルダの変更を監視し、更新が落ち着いたら自動保存する。

use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher as _};
use tauri::{AppHandle, Emitter};

use crate::repo;

/// 最後の変更からこの時間が経過したら自動保存する。
const QUIET_PERIOD: Duration = Duration::from_millis(1500);

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
            let mut relevant = first.is_ok_and(|e| touches_worktree(&root, &e));
            // 変更が続いている間は待つ
            while let Ok(next) = rx.recv_timeout(QUIET_PERIOD) {
                relevant |= next.is_ok_and(|e| touches_worktree(&root, &e));
            }
            if !relevant {
                continue;
            }
            if let Ok(repo) = repo::open_or_init(&root) {
                let _ = repo::save(&repo, "自動保存");
            }
            let _ = app.emit("repo-changed", ());
        }
    });

    Ok(Watcher { _inner: inner })
}
