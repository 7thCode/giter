//! 初心者向けに簡略化した Git 操作。コミットは「保存」、チェックアウトは「戻す」と呼ぶ。

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use git2::build::CheckoutBuilder;
use git2::{
    BranchType, Commit, IndexAddOption, Oid, Reference, Repository, RepositoryInitOptions, Signature, Status,
    StatusOptions, Tree,
};
use serde::{Deserialize, Serialize};

/// OS やアプリが作る一時ファイル(Office の `~$*`、LibreOffice の `.~lock.*#`、vim の `*.swp`)も含める。
const DEFAULT_IGNORE: &str = ".DS_Store\nThumbs.db\ndesktop.ini\n~$*\n.~lock.*#\n*.swp\n";

/// 「本案」にあたるブランチ名を覚えておく設定のキー。
const PRIMARY_KEY: &str = "giter.primary";

/// 本案の表示名。
const PRIMARY_NAME: &str = "本案";

/// これを超えるファイルは、保存前に警告する(GitHub が push を拒否する上限と同じ)。
pub const LARGE_FILE_BYTES: u64 = 100_000_000;

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

/// 「案」(Git のブランチ)。`id` はブランチ名、`name` は画面に出す名前。
#[derive(Debug, Serialize)]
pub struct Plan {
    pub id: String,
    pub name: String,
    pub is_current: bool,
    /// 本案(最初の案)
    pub is_primary: bool,
    /// 最後に保存した時刻(UNIX 秒)
    pub time: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

/// 別案を本案に採用したときに、本案のファイルがどうなるか。
#[derive(Debug, Serialize)]
pub struct PlanChange {
    pub path: String,
    pub kind: ChangeKind,
    /// 本案でも別案でも、同じファイルが別々に変更されている
    pub conflict: bool,
}

/// 競合したファイルの扱い。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Resolution {
    /// 本案の版を使う
    Primary,
    /// 別案の版を使う
    Plan,
    /// 本案の版を残し、別案の版も別名で追加する
    Both,
}

#[derive(Debug, Serialize)]
pub struct LargeFile {
    pub path: String,
    pub size: u64,
    /// まだ一度も保存していないファイル(除外できるのはこれだけ)
    pub is_new: bool,
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
    // ユーザーの git 設定(Windows に多い autocrlf=true)に関わらず、戻したファイルをバイト単位で一致させる
    repo.config()?.set_str("core.autocrlf", "false")?;
    repo.config()?.set_str(PRIMARY_KEY, "main")?;
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

/// いま開いている案のブランチ名(まだ保存が一度もない状態でも取れる)。
fn current_plan_id(repo: &Repository) -> Result<String> {
    let head = repo.find_reference("HEAD")?;
    let target = head.symbolic_target().ok_or_else(|| Error("いま開いている案が分かりません".into()))?;
    Ok(target.strip_prefix("refs/heads/").unwrap_or(target).to_string())
}

/// 本案のブランチ名。以前のバージョンで作ったフォルダには未設定なので、そのとき開いている案を本案とみなす。
fn primary_plan_id(repo: &Repository) -> Result<String> {
    if let Ok(id) = repo.config()?.get_string(PRIMARY_KEY) {
        return Ok(id);
    }
    let id = current_plan_id(repo)?;
    repo.config()?.set_str(PRIMARY_KEY, &id)?;
    Ok(id)
}

fn description_key(id: &str) -> String {
    format!("branch.{id}.description")
}

/// 案の一覧。本案が先頭で、あとは保存が新しい順。
pub fn plans(repo: &Repository) -> Result<Vec<Plan>> {
    let primary = primary_plan_id(repo)?;
    let current = current_plan_id(repo)?;
    let config = repo.config()?;
    let mut out = Vec::new();
    for entry in repo.branches(Some(BranchType::Local))? {
        let (branch, _) = entry?;
        let Some(id) = branch.name()?.map(str::to_string) else { continue };
        let time = branch.get().peel_to_commit().map(|c| c.time().seconds()).unwrap_or(0);
        let is_primary = id == primary;
        let name = if is_primary {
            PRIMARY_NAME.to_string()
        } else {
            config.get_string(&description_key(&id)).unwrap_or_else(|_| id.clone())
        };
        out.push(Plan { is_current: id == current, is_primary, name, id, time });
    }
    out.sort_by(|a, b| b.is_primary.cmp(&a.is_primary).then_with(|| b.time.cmp(&a.time)));
    Ok(out)
}

/// 画面で入力した案の名前を、ブランチ名として使える形にする(空白や記号は `-` に置き換える)。
fn plan_id_from_name(name: &str) -> String {
    let id: String = name
        .chars()
        .map(|c| {
            if c.is_whitespace() || c.is_control() || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\' | '/' | '@') {
                '-'
            } else {
                c
            }
        })
        .collect();
    id.trim_matches('-').to_string()
}

/// 別案を作って、その案に移る。未保存の変更はそのまま新しい案に持っていく(本案は保存した状態のまま残る)。
pub fn create_plan(repo: &Repository, name: &str) -> Result<String> {
    let display = name.trim();
    if display.is_empty() {
        return Err(Error("案の名前を入力してください".into()));
    }
    if display == PRIMARY_NAME {
        return Err(Error(format!("「{PRIMARY_NAME}」は使えない名前です。別の名前にしてください")));
    }
    let id = plan_id_from_name(display);
    if !Reference::is_valid_name(&format!("refs/heads/{id}")) {
        return Err(Error("この名前は使えません。別の名前にしてください".into()));
    }
    if repo.find_branch(&id, BranchType::Local).is_ok() {
        return Err(Error("同じ名前の案がすでにあります".into()));
    }
    let head = head_commit(repo).ok_or_else(|| Error("先に一度保存してください".into()))?;
    repo.branch(&id, &head, false)?;
    repo.config()?.set_str(&description_key(&id), display)?;
    repo.set_head(&format!("refs/heads/{id}"))?;
    Ok(id)
}

/// 別の案に切り替える。フォルダの中身がその案の状態に入れ替わるので、未保存の変更があるときは拒否する。
pub fn switch_plan(repo: &Repository, id: &str) -> Result<()> {
    if current_plan_id(repo)? == id {
        return Ok(());
    }
    if pending_count(repo)? > 0 {
        return Err(Error("未保存の変更があります。先に保存してください".into()));
    }
    let branch = repo
        .find_branch(id, BranchType::Local)
        .map_err(|_| Error(format!("その案が見つかりません: {id}")))?;
    let commit = branch.get().peel_to_commit()?;
    let mut cb = CheckoutBuilder::new();
    cb.safe();
    repo.checkout_tree(commit.tree()?.as_object(), Some(&mut cb))?;
    repo.set_head(&format!("refs/heads/{id}"))?;
    Ok(())
}

fn plan_commit<'r>(repo: &'r Repository, id: &str) -> Result<Commit<'r>> {
    let branch = repo
        .find_branch(id, BranchType::Local)
        .map_err(|_| Error(format!("その案が見つかりません: {id}")))?;
    Ok(branch.get().peel_to_commit()?)
}

fn tree_entry(tree: &Tree, path: &Path) -> Option<Oid> {
    tree.get_path(path).ok().map(|e| e.id())
}

/// 別案を本案に採用すると、本案のファイルがどう変わるか(パス順)。
/// 競合 = 分かれた時点から本案も別案も変更していて、結果が食い違うファイル。
pub fn plan_changes(repo: &Repository, from_id: &str) -> Result<Vec<PlanChange>> {
    let primary = primary_plan_id(repo)?;
    if from_id == primary {
        return Err(Error("本案は、本案に採用できません".into()));
    }
    let ours = plan_commit(repo, &primary)?;
    let theirs = plan_commit(repo, from_id)?;
    let base = repo.find_commit(repo.merge_base(ours.id(), theirs.id())?)?.tree()?;
    let (ours_tree, theirs_tree) = (ours.tree()?, theirs.tree()?);
    let diff = repo.diff_tree_to_tree(Some(&base), Some(&theirs_tree), None)?;
    let mut changes = Vec::new();
    for delta in diff.deltas() {
        let Some(path) = delta.new_file().path().or_else(|| delta.old_file().path()) else { continue };
        let (b, o, t) = (tree_entry(&base, path), tree_entry(&ours_tree, path), tree_entry(&theirs_tree, path));
        if o == t {
            continue; // 本案がすでに同じ内容
        }
        let kind = match (o, t) {
            (_, None) => ChangeKind::Deleted,
            (None, Some(_)) => ChangeKind::Added,
            _ => ChangeKind::Modified,
        };
        changes.push(PlanChange { path: path.to_string_lossy().replace('\\', "/"), kind, conflict: o != b });
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(changes)
}

/// 「両方残す」で別案の版を足すときの名前(`name (案名).ext`、重なれば連番)。
fn both_copy_path(workdir: &Path, rel: &str, plan_name: &str) -> PathBuf {
    let path = Path::new(rel);
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let label = plan_name.replace(['/', '\\'], "-");
    let dir = workdir.join(path.parent().unwrap_or(Path::new("")));
    let first = dir.join(format!("{stem} ({label}){ext}"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| dir.join(format!("{stem} ({label}) {n}{ext}")))
        .find(|c| !c.exists())
        .expect("unbounded range")
}

/// 別案を本案に採用する。本案に移ったうえで、別案で変えたファイルを本案に反映し、
/// 「採用しました」の記録(本案と別案をつなぐ保存)を1件作る。別案はそのまま残る。
/// 競合したファイルは `resolutions` で扱いを指定する。行単位のマージはしない。
pub fn adopt_plan(repo: &Repository, from_id: &str, resolutions: &HashMap<String, Resolution>) -> Result<()> {
    let changes = plan_changes(repo, from_id)?;
    if let Some(c) = changes.iter().find(|c| c.conflict && !resolutions.contains_key(&c.path)) {
        return Err(Error(format!("「{}」をどう扱うか選んでください", c.path)));
    }
    if pending_count(repo)? > 0 {
        return Err(Error("未保存の変更があります。先に保存してください".into()));
    }
    let primary = primary_plan_id(repo)?;
    switch_plan(repo, &primary)?;

    let root = workdir(repo)?.to_path_buf();
    let ours = plan_commit(repo, &primary)?;
    let theirs = plan_commit(repo, from_id)?;
    let theirs_tree = theirs.tree()?;
    let plan_name = repo
        .config()?
        .get_string(&description_key(from_id))
        .unwrap_or_else(|_| from_id.to_string());

    for change in &changes {
        let resolution = if change.conflict { resolutions[&change.path] } else { Resolution::Plan };
        let in_plan = tree_entry(&theirs_tree, Path::new(&change.path)).is_some();
        match resolution {
            Resolution::Primary => {}
            Resolution::Plan if in_plan => {
                let mut cb = CheckoutBuilder::new();
                cb.force().path(&change.path);
                repo.checkout_tree(theirs_tree.as_object(), Some(&mut cb))?;
            }
            Resolution::Plan => {
                let path = safe_join(&root, &change.path)?;
                if path.exists() {
                    fs::remove_file(path)?;
                }
            }
            Resolution::Both if in_plan => {
                let entry = theirs_tree.get_path(Path::new(&change.path))?;
                let blob = repo.find_blob(entry.id())?;
                let dest = both_copy_path(&root, &change.path, &plan_name);
                fs::write(dest, blob.content())?;
            }
            Resolution::Both => {}
        }
    }

    let mut index = repo.index()?;
    index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)?;
    index.update_all(["*"].iter(), None)?;
    index.write()?;
    let tree = repo.find_tree(index.write_tree()?)?;
    let sig = signature(repo)?;
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        &format!("「{plan_name}」を採用しました"),
        &tree,
        &[&ours, &theirs],
    )?;
    Ok(())
}

/// 未保存の変更があるファイル数。
pub fn pending_count(repo: &Repository) -> Result<usize> {
    Ok(changed_files(repo)?.len())
}

/// 未保存の変更のうち、`threshold` バイトを超えるファイル(大きい順)。
pub fn pending_large(repo: &Repository, threshold: u64) -> Result<Vec<LargeFile>> {
    let root = workdir(repo)?;
    let mut found: Vec<LargeFile> = changed_files(repo)?
        .into_iter()
        .filter_map(|(path, status)| {
            let meta = fs::metadata(root.join(&path)).ok()?;
            (meta.is_file() && meta.len() > threshold).then(|| LargeFile {
                size: meta.len(),
                is_new: status == FileStatus::New,
                path,
            })
        })
        .collect();
    found.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
    Ok(found)
}

/// gitignore のパターンとして、パスの文字をそのまま一致させるためのエスケープ。
fn exclude_pattern(path: &str) -> String {
    let mut out = String::from("/");
    for c in path.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '\\' | '#' | '!') {
            out.push('\\');
        }
        out.push(c);
    }
    // 末尾の空白は gitignore では無視されるので、エスケープして残す
    if out.ends_with(' ') {
        out.insert(out.len() - 1, '\\');
    }
    out
}

/// 以後保存の対象にしない。ユーザーの `.gitignore` は触らず、`.git/info/exclude` に追記する。
/// すでに保存済み(追跡中)のファイルには効かないので、新規ファイルにだけ使う。
pub fn exclude_paths(repo: &Repository, paths: &[String]) -> Result<()> {
    let root = workdir(repo)?;
    let mut lines = String::new();
    for path in paths {
        safe_join(root, path)?;
        if path.contains('\n') {
            return Err(Error(format!("除外できないファイル名です: {path}")));
        }
        lines.push_str(&exclude_pattern(path));
        lines.push('\n');
    }
    let info = repo.path().join("info");
    fs::create_dir_all(&info)?;
    let file = info.join("exclude");
    let mut content = fs::read_to_string(&file).unwrap_or_default();
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&lines);
    fs::write(&file, content)?;
    Ok(())
}

fn dir_size(dir: &Path) -> io::Result<u64> {
    let mut total = 0;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        total += if meta.is_dir() { dir_size(&entry.path())? } else { meta.len() };
    }
    Ok(total)
}

/// 履歴(`.git`)がディスク上で使っているバイト数。
pub fn history_size(repo: &Repository) -> Result<u64> {
    Ok(dir_size(repo.path())?)
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
    let dir = export_dir().join(&id[..8.min(id.len())]);
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

/// 過去の版をドラッグ取り出し用に書き出す一時フォルダ。
fn export_dir() -> PathBuf {
    std::env::temp_dir().join("giter-export")
}

/// ドロップされたパスが、アプリ自身が出したドラッグ(=コピーしてはいけないもの)か。
/// ウィンドウ内でファイルをドラッグしてドロップしても、OS からは外部のドロップと区別できない。
/// - 管理フォルダの中: 同じフォルダ内の移動の操作であり、コピーすると重複する
/// - 管理フォルダを含む親: コピー先が自分自身の中になり、増え続ける
/// - 過去の版の書き出し先: 取り出し用の一時ファイルで、追加するものではない
fn is_own_drag(src: &Path, root: &Path) -> bool {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let (src, root, export) = (canon(src), canon(root), canon(&export_dir()));
    src.starts_with(&root) || root.starts_with(&src) || src.starts_with(&export)
}

/// 外部のファイル/フォルダを管理フォルダ内の `dest_rel` にコピーして追加する。元は残す。
/// アプリ自身が出したドラッグ(管理フォルダ内のファイルなど)は無視する。追加した項目のパスを返す。
pub fn add_files(repo: &Repository, dest_rel: &str, sources: &[PathBuf]) -> Result<Vec<String>> {
    let root = workdir(repo)?;
    let dest_dir = safe_join(root, dest_rel)?;
    if !dest_dir.is_dir() {
        return Err(Error(format!("追加先のフォルダがありません: {dest_rel}")));
    }
    let mut added = Vec::new();
    for src in sources.iter().filter(|s| !is_own_drag(s, root)) {
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
    fn office_and_editor_temp_files_are_ignored() {
        let (d, repo) = setup();
        write(&d, "~$report.docx", "lock");
        write(&d, ".~lock.report.odt#", "lock");
        write(&d, "notes.txt.swp", "swap");
        assert_eq!(pending_count(&repo).unwrap(), 0);
        write(&d, "report.docx", "real");
        assert_eq!(pending_count(&repo).unwrap(), 1);
    }

    #[test]
    fn line_endings_are_not_converted() {
        let (_d, repo) = setup();
        assert_eq!(repo.config().unwrap().get_string("core.autocrlf").unwrap(), "false");
    }

    #[test]
    fn binary_and_crlf_files_restore_byte_for_byte() {
        let (d, repo) = setup();
        let binary: Vec<u8> = (0..=255u8).chain([0, 0, 13, 10, 13, 10]).collect();
        let text = b"line1\r\nline2\r\n".to_vec();
        fs::write(d.path().join("data.bin"), &binary).unwrap();
        fs::write(d.path().join("text.txt"), &text).unwrap();
        let v1 = save(&repo, "v1").unwrap().unwrap();
        fs::write(d.path().join("data.bin"), b"changed").unwrap();
        fs::write(d.path().join("text.txt"), b"changed").unwrap();
        save(&repo, "v2").unwrap();
        restore(&repo, &v1, None).unwrap();
        assert_eq!(fs::read(d.path().join("data.bin")).unwrap(), binary);
        assert_eq!(fs::read(d.path().join("text.txt")).unwrap(), text);
    }

    #[test]
    fn pending_large_lists_only_files_over_threshold() {
        let (d, repo) = setup();
        write(&d, "small.txt", "x");
        write(&d, "big.bin", &"x".repeat(50));
        write(&d, "sub/bigger.bin", &"x".repeat(80));
        let large = pending_large(&repo, 10).unwrap();
        let found: Vec<_> = large.iter().map(|f| (f.path.as_str(), f.size, f.is_new)).collect();
        assert_eq!(found, [("sub/bigger.bin", 80, true), ("big.bin", 50, true)]);
        // 保存済みのファイルが大きくなった場合は is_new が false
        save(&repo, "s").unwrap();
        write(&d, "big.bin", &"y".repeat(60));
        let large = pending_large(&repo, 10).unwrap();
        assert_eq!(large.len(), 1);
        assert!(!large[0].is_new);
    }

    #[test]
    fn excluded_paths_are_no_longer_pending() {
        let (d, repo) = setup();
        write(&d, "[a]*.bin", "special");
        write(&d, "#x", "hash");
        write(&d, "a.bin", "plain");
        write(&d, "keep.txt", "keep");
        exclude_paths(&repo, &["[a]*.bin".to_string(), "#x".to_string()]).unwrap();
        // glob と解釈されると a.bin も巻き込まれる。文字どおりの一致だけが除外される
        assert_eq!(pending_count(&repo).unwrap(), 2);
        save(&repo, "s").unwrap();
        assert_eq!(pending_count(&repo).unwrap(), 0);
        // 除外したファイルは履歴に入らず、そうでないファイルは入る(ディスク上には残る)
        assert!(history(&repo, Some("[a]*.bin")).unwrap().is_empty());
        assert!(history(&repo, Some("#x")).unwrap().is_empty());
        assert_eq!(history(&repo, Some("a.bin")).unwrap().len(), 1);
        assert!(d.path().join("#x").exists());
    }

    #[test]
    fn exclude_paths_rejects_escaping_paths() {
        let (_d, repo) = setup();
        assert!(exclude_paths(&repo, &["../outside".to_string()]).is_err());
    }

    #[test]
    fn history_size_grows_after_saving() {
        let (d, repo) = setup();
        let before = history_size(&repo).unwrap();
        write(&d, "data.bin", &"z".repeat(5000));
        save(&repo, "s").unwrap();
        assert!(history_size(&repo).unwrap() > before);
    }

    #[test]
    fn dropping_a_managed_file_back_into_the_window_is_ignored() {
        let (d, repo) = setup();
        write(&d, "doc.txt", "x");
        write(&d, "sub/inner.txt", "y");
        // 管理フォルダ内のファイル/フォルダをドロップ(ファイル一覧内のドラッグ&ドロップ)
        let added = add_files(&repo, "", &[d.path().join("doc.txt"), d.path().join("sub")]).unwrap();
        assert!(added.is_empty(), "{added:?}");
        assert!(!d.path().join("doc 2.txt").exists());
        assert!(!d.path().join("sub 2").exists());
        // 別のフォルダへドロップしても、コピーしない
        let added = add_files(&repo, "sub", &[d.path().join("doc.txt")]).unwrap();
        assert!(added.is_empty());
        assert!(!d.path().join("sub/doc.txt").exists());
    }

    #[test]
    fn dropping_the_managed_folder_or_its_parent_is_ignored() {
        let (d, repo) = setup();
        let parent = d.path().parent().unwrap().to_path_buf();
        let added = add_files(&repo, "", &[d.path().to_path_buf(), parent]).unwrap();
        assert!(added.is_empty(), "{added:?}");
    }

    #[test]
    fn dropping_an_exported_version_is_ignored() {
        let (d, repo) = setup();
        write(&d, "a.txt", "old");
        let old = save(&repo, "old").unwrap().unwrap();
        let exported = export_version(&repo, &old, "a.txt").unwrap();
        assert!(add_files(&repo, "", &[exported]).unwrap().is_empty());
        assert!(!d.path().join("a 2.txt").exists());
    }

    #[test]
    fn external_files_are_still_added_alongside_ignored_ones() {
        let (d, repo) = setup();
        write(&d, "own.txt", "x");
        let ext = TempDir::new().unwrap();
        fs::write(ext.path().join("outside.txt"), "y").unwrap();
        let added = add_files(&repo, "", &[d.path().join("own.txt"), ext.path().join("outside.txt")]).unwrap();
        assert_eq!(added, ["outside.txt"]);
    }

    fn current(repo: &Repository) -> Plan {
        plans(repo).unwrap().into_iter().find(|p| p.is_current).unwrap()
    }

    fn tip(repo: &Repository, id: &str) -> Oid {
        repo.find_branch(id, BranchType::Local).unwrap().get().peel_to_commit().unwrap().id()
    }

    #[test]
    fn a_new_folder_has_only_the_primary_plan() {
        let (_d, repo) = setup();
        let list = plans(&repo).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].name.as_str(), list[0].is_primary, list[0].is_current), ("本案", true, true));
    }

    #[test]
    fn creating_a_plan_takes_unsaved_changes_along_and_leaves_the_primary_alone() {
        let (d, repo) = setup();
        write(&d, "a.txt", "1");
        save(&repo, "v1").unwrap();
        let primary_tip = tip(&repo, "main");
        write(&d, "a.txt", "2"); // 未保存
        let id = create_plan(&repo, "値上げ案").unwrap();
        let now = current(&repo);
        assert_eq!((now.id.as_str(), now.name.as_str(), now.is_primary), (id.as_str(), "値上げ案", false));
        assert_eq!(pending_count(&repo).unwrap(), 1, "未保存の変更は新しい案に持っていく");
        save(&repo, "plan v2").unwrap();
        assert_eq!(tip(&repo, "main"), primary_tip, "本案は動かない");
        assert_ne!(tip(&repo, &id), primary_tip);
    }

    #[test]
    fn switching_swaps_the_folder_contents() {
        let (d, repo) = setup();
        write(&d, "a.txt", "1");
        save(&repo, "v1").unwrap();
        let id = create_plan(&repo, "別案").unwrap();
        write(&d, "a.txt", "2");
        write(&d, "only-in-plan.txt", "x");
        save(&repo, "plan work").unwrap();

        switch_plan(&repo, "main").unwrap();
        assert_eq!(fs::read_to_string(d.path().join("a.txt")).unwrap(), "1");
        assert!(!d.path().join("only-in-plan.txt").exists());
        assert!(current(&repo).is_primary);

        switch_plan(&repo, &id).unwrap();
        assert_eq!(fs::read_to_string(d.path().join("a.txt")).unwrap(), "2");
        assert!(d.path().join("only-in-plan.txt").exists());
        assert_eq!(pending_count(&repo).unwrap(), 0);
    }

    #[test]
    fn switching_with_unsaved_changes_is_refused_and_changes_are_kept() {
        let (d, repo) = setup();
        write(&d, "a.txt", "1");
        save(&repo, "v1").unwrap();
        let id = create_plan(&repo, "別案").unwrap();
        write(&d, "a.txt", "unsaved");
        assert!(switch_plan(&repo, "main").is_err());
        assert_eq!(current(&repo).id, id);
        assert_eq!(fs::read_to_string(d.path().join("a.txt")).unwrap(), "unsaved");
    }

    #[test]
    fn plan_names_are_validated() {
        let (_d, repo) = setup();
        assert!(create_plan(&repo, "   ").is_err());
        assert!(create_plan(&repo, "本案").is_err());
        create_plan(&repo, "春の 案/改").unwrap();
        // 空白や / はブランチ名では使えないが、画面では入力どおりの名前を出す
        let now = current(&repo);
        assert_eq!(now.name, "春の 案/改");
        assert_eq!(now.id, "春の-案-改");
        switch_plan(&repo, "main").unwrap();
        assert!(create_plan(&repo, "春の 案/改").is_err(), "同名は作れない");
        assert!(create_plan(&repo, "春の-案-改").is_err());
    }

    #[test]
    fn history_follows_the_current_plan_only() {
        let (d, repo) = setup();
        write(&d, "a.txt", "1");
        save(&repo, "共通").unwrap();
        let id = create_plan(&repo, "別案").unwrap();
        write(&d, "a.txt", "2");
        save(&repo, "別案での作業").unwrap();
        let msgs = |repo: &Repository| history(repo, None).unwrap().into_iter().map(|v| v.message).collect::<Vec<_>>();
        assert_eq!(msgs(&repo)[0], "別案での作業");
        switch_plan(&repo, "main").unwrap();
        assert_eq!(msgs(&repo)[0], "共通", "別案の作業は本案の履歴に出ない");
        switch_plan(&repo, &id).unwrap();
        assert!(msgs(&repo).contains(&"共通".to_string()));
    }

    #[test]
    fn the_primary_is_remembered_for_folders_made_by_older_versions() {
        let (_d, repo) = setup();
        repo.config().unwrap().remove(PRIMARY_KEY).unwrap();
        let list = plans(&repo).unwrap();
        assert!(list[0].is_primary);
        assert_eq!(repo.config().unwrap().get_string(PRIMARY_KEY).unwrap(), "main");
    }

    fn names(changes: &[PlanChange]) -> Vec<(String, ChangeKind, bool)> {
        changes.iter().map(|c| (c.path.clone(), c.kind, c.conflict)).collect()
    }

    #[test]
    fn adopting_lists_added_modified_deleted_files() {
        let (d, repo) = setup();
        write(&d, "keep.txt", "k");
        write(&d, "edit.txt", "1");
        write(&d, "gone.txt", "g");
        save(&repo, "v1").unwrap();
        let id = create_plan(&repo, "別案").unwrap();
        write(&d, "edit.txt", "2");
        write(&d, "new.txt", "n");
        fs::remove_file(d.path().join("gone.txt")).unwrap();
        save(&repo, "plan").unwrap();
        assert_eq!(
            names(&plan_changes(&repo, &id).unwrap()),
            vec![
                ("edit.txt".into(), ChangeKind::Modified, false),
                ("gone.txt".into(), ChangeKind::Deleted, false),
                ("new.txt".into(), ChangeKind::Added, false),
            ]
        );
        assert!(plan_changes(&repo, "main").is_err(), "本案は採用できない");
    }

    #[test]
    fn adopting_applies_plan_to_primary_and_keeps_the_plan() {
        let (d, repo) = setup();
        write(&d, "edit.txt", "1");
        write(&d, "gone.txt", "g");
        save(&repo, "v1").unwrap();
        let id = create_plan(&repo, "値上げ案").unwrap();
        write(&d, "edit.txt", "2");
        write(&d, "new.txt", "n");
        fs::remove_file(d.path().join("gone.txt")).unwrap();
        save(&repo, "plan").unwrap();
        let plan_tip = tip(&repo, &id);

        adopt_plan(&repo, &id, &HashMap::new()).unwrap();
        assert!(current(&repo).is_primary, "採用後は本案を開く");
        assert_eq!(fs::read_to_string(d.path().join("edit.txt")).unwrap(), "2");
        assert!(d.path().join("new.txt").exists());
        assert!(!d.path().join("gone.txt").exists());
        assert_eq!(pending_count(&repo).unwrap(), 0);
        assert_eq!(tip(&repo, &id), plan_tip, "別案は残る");
        assert_eq!(history(&repo, None).unwrap()[0].message, "「値上げ案」を採用しました");
        assert!(plan_changes(&repo, &id).unwrap().is_empty(), "採用後は差がない");
    }

    #[test]
    fn conflicts_need_a_choice_and_each_choice_works() {
        for (choice, edit, extra) in [
            (Resolution::Primary, "primary", None),
            (Resolution::Plan, "plan", None),
            (Resolution::Both, "primary", Some("plan")),
        ] {
            let (d, repo) = setup();
            write(&d, "a.txt", "base");
            save(&repo, "v1").unwrap();
            let id = create_plan(&repo, "案").unwrap();
            write(&d, "a.txt", "plan");
            save(&repo, "plan").unwrap();
            switch_plan(&repo, "main").unwrap();
            write(&d, "a.txt", "primary");
            save(&repo, "primary").unwrap();

            let changes = plan_changes(&repo, &id).unwrap();
            assert_eq!(names(&changes), vec![("a.txt".into(), ChangeKind::Modified, true)]);
            assert!(adopt_plan(&repo, &id, &HashMap::new()).is_err(), "選ばずに採用はできない");
            assert_eq!(pending_count(&repo).unwrap(), 0);

            adopt_plan(&repo, &id, &HashMap::from([("a.txt".to_string(), choice)])).unwrap();
            assert_eq!(fs::read_to_string(d.path().join("a.txt")).unwrap(), edit);
            let copy = d.path().join("a (案).txt");
            assert_eq!(copy.exists(), extra.is_some());
            if let Some(content) = extra {
                assert_eq!(fs::read_to_string(copy).unwrap(), content);
            }
            assert_eq!(pending_count(&repo).unwrap(), 0);
        }
    }

    #[test]
    fn adopting_with_unsaved_changes_is_refused() {
        let (d, repo) = setup();
        write(&d, "a.txt", "1");
        save(&repo, "v1").unwrap();
        let id = create_plan(&repo, "案").unwrap();
        write(&d, "a.txt", "2");
        save(&repo, "plan").unwrap();
        write(&d, "a.txt", "3");
        assert!(adopt_plan(&repo, &id, &HashMap::new()).is_err());
        assert_eq!(fs::read_to_string(d.path().join("a.txt")).unwrap(), "3");
    }

    #[test]
    fn parent_dir_paths_are_rejected() {
        let (_d, repo) = setup();
        assert!(list_dir(&repo, "../").is_err());
        assert!(restore(&repo, &history(&repo, None).unwrap()[0].id, Some("../x")).is_err());
    }
}
