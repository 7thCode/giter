//! 初心者向けに簡略化した Git 操作。コミットは「保存」、チェックアウトは「戻す」と呼ぶ。

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use git2::build::CheckoutBuilder;
use git2::{Commit, IndexAddOption, Oid, Repository, RepositoryInitOptions, Signature, Status, StatusOptions};
use serde::Serialize;

const DEFAULT_IGNORE: &str = ".DS_Store\nThumbs.db\ndesktop.ini\n";

#[derive(Debug)]
pub struct Error(pub String);

impl From<git2::Error> for Error {
    fn from(e: git2::Error) -> Self {
        Error(e.message().to_string())
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error(e.to_string())
    }
}

impl Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Saved,
    New,
    Modified,
}

#[derive(Debug, Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    /// UNIX 秒
    pub modified: i64,
    pub status: FileStatus,
}

#[derive(Debug, Serialize)]
pub struct Version {
    pub id: String,
    pub message: String,
    /// UNIX 秒
    pub time: i64,
}

/// フォルダを開く。Git 管理下でなければ初期化し、最初の状態を保存する。
pub fn open_or_init(root: &Path) -> Result<Repository> {
    if let Ok(repo) = Repository::open(root) {
        return Ok(repo);
    }
    let mut opts = RepositoryInitOptions::new();
    opts.initial_head("main");
    let repo = Repository::init_opts(root, &opts)?;
    let ignore = root.join(".gitignore");
    if !ignore.exists() {
        fs::write(&ignore, DEFAULT_IGNORE)?;
    }
    save(&repo, "最初の状態")?;
    Ok(repo)
}

fn signature(repo: &Repository) -> Result<Signature<'static>> {
    Ok(repo
        .signature()
        .or_else(|_| Signature::now("giter", "giter@localhost"))?)
}

fn head_commit(repo: &Repository) -> Option<Commit<'_>> {
    repo.head().ok()?.peel_to_commit().ok()
}

/// 親ディレクトリへ抜ける指定を拒否して、管理フォルダ内のパスに解決する。
fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    let rel_path = Path::new(rel);
    let escapes = rel_path
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_)));
    if escapes {
        return Err(Error(format!("管理フォルダの外は指定できません: {rel}")));
    }
    Ok(root.join(rel_path))
}

fn workdir(repo: &Repository) -> Result<&Path> {
    repo.workdir().ok_or_else(|| Error("作業フォルダがありません".into()))
}

/// すべての変更を保存する。変更がなければ `None`。
pub fn save(repo: &Repository, message: &str) -> Result<Option<String>> {
    commit_all(repo, message, false)
}

/// 今の内容と同じ版がすでに履歴にあれば保存しない(失う内容がないため)。
/// 戻す前の安全保存に使い、戻した状態の再保存で履歴が増えるのを防ぐ。
fn save_unless_known(repo: &Repository, message: &str) -> Result<Option<String>> {
    commit_all(repo, message, true)
}

fn commit_all(repo: &Repository, message: &str, skip_if_known: bool) -> Result<Option<String>> {
    let mut index = repo.index()?;
    index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)?;
    index.update_all(["*"].iter(), None)?;
    index.write()?;
    let tree_id = index.write_tree()?;

    let parent = head_commit(repo);
    if parent.as_ref().is_some_and(|p| p.tree_id() == tree_id) {
        return Ok(None);
    }
    if skip_if_known && parent.is_some() {
        let mut walk = repo.revwalk()?;
        walk.push_head()?;
        for oid in walk {
            if repo.find_commit(oid?)?.tree_id() == tree_id {
                return Ok(None);
            }
        }
    }
    let tree = repo.find_tree(tree_id)?;
    let sig = signature(repo)?;
    let parents: Vec<&Commit> = parent.iter().collect();
    let oid = repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)?;
    Ok(Some(oid.to_string()))
}

/// 未保存の変更があるファイルの一覧(パスは `/` 区切り)。
fn changed_files(repo: &Repository) -> Result<HashMap<String, FileStatus>> {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true).include_ignored(false);
    let statuses = repo.statuses(Some(&mut opts))?;
    let mut map = HashMap::new();
    for entry in statuses.iter() {
        let Some(path) = entry.path() else { continue };
        let s = entry.status();
        let status = if s.intersects(Status::WT_NEW | Status::INDEX_NEW) {
            FileStatus::New
        } else {
            FileStatus::Modified
        };
        map.insert(path.trim_end_matches('/').to_string(), status);
    }
    Ok(map)
}

/// 未保存の変更があるファイル数。
pub fn pending_count(repo: &Repository) -> Result<usize> {
    Ok(changed_files(repo)?.len())
}

/// Finder のようにフォルダ直下の項目を返す。`.` で始まる項目は隠す。
pub fn list_dir(repo: &Repository, rel: &str) -> Result<Vec<Entry>> {
    let root = workdir(repo)?;
    let dir = safe_join(root, rel)?;
    let changed = changed_files(repo)?;
    let mut entries = Vec::new();
    for item in fs::read_dir(&dir)? {
        let item = item?;
        let name = item.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let meta = item.metadata()?;
        let path = if rel.is_empty() { name.clone() } else { format!("{}/{}", rel.trim_end_matches('/'), name) };
        let is_dir = meta.is_dir();
        let status = if is_dir {
            let prefix = format!("{path}/");
            let inside: Vec<FileStatus> =
                changed.iter().filter(|(p, _)| p.starts_with(&prefix)).map(|(_, s)| *s).collect();
            if inside.is_empty() {
                FileStatus::Saved
            } else if inside.iter().all(|s| *s == FileStatus::New) {
                FileStatus::New
            } else {
                FileStatus::Modified
            }
        } else {
            changed.get(&path).copied().unwrap_or(FileStatus::Saved)
        };
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64);
        entries.push(Entry { name, path, is_dir, size: if is_dir { 0 } else { meta.len() }, modified, status });
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(entries)
}

/// 保存の履歴(新しい順)。`rel` を指定するとそのファイル/フォルダに変更があったものだけ返す。
pub fn history(repo: &Repository, rel: Option<&str>) -> Result<Vec<Version>> {
    if head_commit(repo).is_none() {
        return Ok(Vec::new());
    }
    let mut walk = repo.revwalk()?;
    walk.push_head()?;
    walk.set_sorting(git2::Sort::TIME)?;
    let mut versions = Vec::new();
    for oid in walk {
        let commit = repo.find_commit(oid?)?;
        if let Some(rel) = rel {
            let entry_id = |c: &Commit| c.tree().ok().and_then(|t| t.get_path(Path::new(rel)).ok()).map(|e| e.id());
            let parent = commit.parents().next();
            if entry_id(&commit) == parent.as_ref().and_then(entry_id) {
                continue;
            }
        }
        versions.push(Version {
            id: commit.id().to_string(),
            message: commit.message().unwrap_or("").trim().to_string(),
            time: commit.time().seconds(),
        });
    }
    Ok(versions)
}

fn find_commit<'r>(repo: &'r Repository, id: &str) -> Result<Commit<'r>> {
    Ok(repo.find_commit(Oid::from_str(id)?)?)
}

/// 指定した時点に戻す。`rel` があればそのファイル/フォルダだけ、なければ全体。
/// 戻す前に未保存の変更があれば自動で保存する(履歴は消さない)。
/// 戻した結果は保存しない。差分は「変更あり」として残り、保存するかはユーザーが決める。
pub fn restore(repo: &Repository, id: &str, rel: Option<&str>) -> Result<()> {
    let target = find_commit(repo, id)?;
    save_unless_known(repo, "戻す前の自動保存")?;
    let tree = target.tree()?;
    let mut cb = CheckoutBuilder::new();
    cb.force();
    match rel {
        Some(rel) => {
            let root = workdir(repo)?;
            let path = safe_join(root, rel)?;
            if tree.get_path(Path::new(rel)).is_ok() {
                cb.path(rel);
                repo.checkout_tree(tree.as_object(), Some(&mut cb))?;
            } else if path.is_dir() {
                fs::remove_dir_all(&path)?;
            } else if path.exists() {
                fs::remove_file(&path)?;
            }
        }
        None => {
            cb.remove_untracked(true);
            repo.checkout_tree(tree.as_object(), Some(&mut cb))?;
        }
    }
    Ok(())
}

/// 管理フォルダ内の項目の絶対パス(外へ抜けるパスは拒否)。
pub fn resolve(repo: &Repository, rel: &str) -> Result<PathBuf> {
    safe_join(workdir(repo)?, rel)
}

/// 過去バージョンのファイルを一時フォルダに書き出し、そのパスを返す(Finder へのドラッグ用)。
pub fn export_version(repo: &Repository, id: &str, rel: &str) -> Result<PathBuf> {
    let commit = find_commit(repo, id)?;
    let entry = commit
        .tree()?
        .get_path(Path::new(rel))
        .map_err(|_| Error(format!("その時点には {rel} がありません")))?;
    let blob = repo
        .find_blob(entry.id())
        .map_err(|_| Error("フォルダは書き出せません。ファイルを選んでください".into()))?;
    let name = Path::new(rel).file_name().ok_or_else(|| Error("ファイル名が不正です".into()))?;
    let dir = std::env::temp_dir().join("giter-export").join(&id[..8.min(id.len())]);
    fs::create_dir_all(&dir)?;
    let out = dir.join(name);
    fs::write(&out, blob.content())?;
    Ok(out)
}

/// Finder と同様、同名があれば「name 2」のように連番を付ける。
fn unique_dest(dir: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..)
        .map(|n| dir.join(format!("{stem} {n}{ext}")))
        .find(|c| !c.exists())
        .expect("unbounded range")
}

fn copy_recursive(src: &Path, dest: &Path) -> io::Result<()> {
    if src.is_dir() {
        fs::create_dir_all(dest)?;
        for item in fs::read_dir(src)? {
            let item = item?;
            if item.file_name() == ".git" {
                continue;
            }
            copy_recursive(&item.path(), &dest.join(item.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(src, dest).map(|_| ())
    }
}

/// 外部のファイル/フォルダを管理フォルダ内の `dest_rel` にコピーして追加する。元は残す。
pub fn add_files(repo: &Repository, dest_rel: &str, sources: &[PathBuf]) -> Result<Vec<String>> {
    let root = workdir(repo)?;
    let dest_dir = safe_join(root, dest_rel)?;
    if !dest_dir.is_dir() {
        return Err(Error(format!("追加先のフォルダがありません: {dest_rel}")));
    }
    let mut added = Vec::new();
    for src in sources {
        let name = src.file_name().ok_or_else(|| Error(format!("名前を取得できません: {}", src.display())))?;
        let dest = unique_dest(&dest_dir, name);
        copy_recursive(src, &dest)?;
        let rel = dest.strip_prefix(root).unwrap_or(&dest);
        added.push(rel.to_string_lossy().replace('\\', "/"));
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Repository) {
        let dir = TempDir::new().unwrap();
        let repo = open_or_init(dir.path()).unwrap();
        (dir, repo)
    }

    fn write(dir: &TempDir, rel: &str, body: &str) {
        let p = dir.path().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    #[test]
    fn init_creates_first_snapshot() {
        let (_d, repo) = setup();
        let h = history(&repo, None).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].message, "最初の状態");
    }

    #[test]
    fn status_badges_follow_changes() {
        let (d, repo) = setup();
        write(&d, "a.txt", "1");
        let list = list_dir(&repo, "").unwrap();
        assert_eq!(list.iter().find(|e| e.name == "a.txt").unwrap().status, FileStatus::New);
        save(&repo, "a").unwrap();
        let list = list_dir(&repo, "").unwrap();
        assert_eq!(list.iter().find(|e| e.name == "a.txt").unwrap().status, FileStatus::Saved);
        write(&d, "a.txt", "2");
        let list = list_dir(&repo, "").unwrap();
        assert_eq!(list.iter().find(|e| e.name == "a.txt").unwrap().status, FileStatus::Modified);
        assert_eq!(pending_count(&repo).unwrap(), 1);
    }

    #[test]
    fn hidden_files_and_git_dir_are_not_listed() {
        let (_d, repo) = setup();
        let names: Vec<_> = list_dir(&repo, "").unwrap().into_iter().map(|e| e.name).collect();
        assert!(names.is_empty(), "{names:?}");
    }

    #[test]
    fn folder_status_reflects_children() {
        let (d, repo) = setup();
        write(&d, "sub/x.txt", "1");
        let list = list_dir(&repo, "").unwrap();
        let sub = list.iter().find(|e| e.name == "sub").unwrap();
        assert!(sub.is_dir);
        assert_eq!(sub.status, FileStatus::New);
    }

    #[test]
    fn save_without_changes_returns_none() {
        let (_d, repo) = setup();
        assert!(save(&repo, "x").unwrap().is_none());
    }

    #[test]
    fn file_history_only_lists_touching_versions() {
        let (d, repo) = setup();
        write(&d, "a.txt", "1");
        save(&repo, "a1").unwrap();
        write(&d, "b.txt", "1");
        save(&repo, "b1").unwrap();
        write(&d, "a.txt", "2");
        save(&repo, "a2").unwrap();
        let msgs: Vec<_> = history(&repo, Some("a.txt")).unwrap().into_iter().map(|v| v.message).collect();
        assert_eq!(msgs, ["a2", "a1"]);
    }

    #[test]
    fn restore_file_keeps_history_and_autosaves_first() {
        let (d, repo) = setup();
        write(&d, "a.txt", "old");
        let old = save(&repo, "old").unwrap().unwrap();
        write(&d, "a.txt", "new-unsaved");
        restore(&repo, &old, Some("a.txt")).unwrap();
        assert_eq!(fs::read_to_string(d.path().join("a.txt")).unwrap(), "old");
        let msgs: Vec<_> = history(&repo, None).unwrap().into_iter().map(|v| v.message).collect();
        assert!(msgs.iter().any(|m| m == "戻す前の自動保存"), "{msgs:?}");
        // 戻す前の内容も履歴に残っているので、さらに取り戻せる
        let backup = history(&repo, None).unwrap().into_iter().find(|v| v.message == "戻す前の自動保存").unwrap();
        restore(&repo, &backup.id, Some("a.txt")).unwrap();
        assert_eq!(fs::read_to_string(d.path().join("a.txt")).unwrap(), "new-unsaved");
    }

    #[test]
    fn restore_removes_file_absent_in_target() {
        let (d, repo) = setup();
        let first = history(&repo, None).unwrap()[0].id.clone();
        write(&d, "later.txt", "x");
        save(&repo, "later").unwrap();
        restore(&repo, &first, Some("later.txt")).unwrap();
        assert!(!d.path().join("later.txt").exists());
    }

    #[test]
    fn restore_all_resets_folder() {
        let (d, repo) = setup();
        write(&d, "a.txt", "1");
        let v1 = save(&repo, "v1").unwrap().unwrap();
        write(&d, "a.txt", "2");
        write(&d, "b.txt", "extra");
        restore(&repo, &v1, None).unwrap();
        assert_eq!(fs::read_to_string(d.path().join("a.txt")).unwrap(), "1");
        assert!(!d.path().join("b.txt").exists());
        // 戻した結果は保存されず、変更として残る
        assert!(pending_count(&repo).unwrap() > 0);
    }

    #[test]
    fn restore_does_not_add_revert_entries_to_history() {
        let (d, repo) = setup();
        write(&d, "a.txt", "1");
        let v1 = save(&repo, "v1").unwrap().unwrap();
        write(&d, "a.txt", "2");
        save(&repo, "v2").unwrap();
        let before = history(&repo, None).unwrap().len();

        // 未保存の変更がなければ、戻しても履歴は増えない(全体・ファイル単位とも)
        restore(&repo, &v1, None).unwrap();
        restore(&repo, &v1, Some("a.txt")).unwrap();
        assert_eq!(history(&repo, None).unwrap().len(), before);
        assert_eq!(fs::read_to_string(d.path().join("a.txt")).unwrap(), "1");

        // 保存するかはユーザー次第。保存して初めて履歴に載り、メッセージは指定したもの
        save(&repo, "v1に戻した").unwrap();
        let msgs: Vec<_> = history(&repo, None).unwrap().into_iter().map(|v| v.message).collect();
        assert_eq!(msgs.len(), before + 1);
        assert!(msgs.iter().all(|m| !m.contains("戻しました")), "{msgs:?}");
    }

    #[test]
    fn export_version_writes_old_content() {
        let (d, repo) = setup();
        write(&d, "a.txt", "old");
        let old = save(&repo, "old").unwrap().unwrap();
        write(&d, "a.txt", "new");
        let out = export_version(&repo, &old, "a.txt").unwrap();
        assert_eq!(fs::read_to_string(out).unwrap(), "old");
    }

    #[test]
    fn add_files_copies_and_avoids_overwrite() {
        let (d, repo) = setup();
        let ext = TempDir::new().unwrap();
        fs::write(ext.path().join("doc.txt"), "hello").unwrap();
        let first = add_files(&repo, "", &[ext.path().join("doc.txt")]).unwrap();
        let second = add_files(&repo, "", &[ext.path().join("doc.txt")]).unwrap();
        assert_eq!(first, ["doc.txt"]);
        assert_eq!(second, ["doc 2.txt"]);
        assert!(ext.path().join("doc.txt").exists(), "元ファイルは残る");
        assert!(d.path().join("doc 2.txt").exists());
    }

    #[test]
    fn add_files_copies_directories() {
        let (d, repo) = setup();
        let ext = TempDir::new().unwrap();
        fs::create_dir_all(ext.path().join("proj/inner")).unwrap();
        fs::write(ext.path().join("proj/inner/f.txt"), "x").unwrap();
        let added = add_files(&repo, "", &[ext.path().join("proj")]).unwrap();
        assert_eq!(added.len(), 1);
        assert!(d.path().join(&added[0]).join("inner/f.txt").exists());
    }

    #[test]
    fn parent_dir_paths_are_rejected() {
        let (_d, repo) = setup();
        assert!(list_dir(&repo, "../").is_err());
        assert!(restore(&repo, &history(&repo, None).unwrap()[0].id, Some("../x")).is_err());
    }
}
