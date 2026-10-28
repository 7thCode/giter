mod repo;
mod watcher;

use std::path::PathBuf;
use std::sync::Mutex;

use repo::{Entry, Error, Result, Version};
use tauri::{AppHandle, State};

#[derive(Default)]
struct AppState {
    watcher: Mutex<Option<watcher::Watcher>>,
}

/// フォルダを管理対象にして監視を開始する。未管理なら初期化する。
#[tauri::command(async)]
fn open_folder(app: AppHandle, state: State<AppState>, root: String) -> Result<()> {
    let root = PathBuf::from(root);
    repo::open_or_init(&root)?;
    let w = watcher::start(app, root).map_err(|e| Error(e.to_string()))?;
    *state.watcher.lock().map_err(|_| Error("内部エラー".into()))? = Some(w);
    Ok(())
}

#[tauri::command(async)]
fn list_dir(root: String, rel: String) -> Result<Vec<Entry>> {
    repo::list_dir(&repo::open_or_init(&PathBuf::from(root))?, &rel)
}

#[tauri::command(async)]
fn pending_count(root: String) -> Result<usize> {
    repo::pending_count(&repo::open_or_init(&PathBuf::from(root))?)
}

/// 「いま保存」。変更がなければ `false`。
#[tauri::command(async)]
fn save_now(root: String, message: String) -> Result<bool> {
    let message = message.trim();
    let message = if message.is_empty() { "保存" } else { message };
    Ok(repo::save(&repo::open_or_init(&PathBuf::from(root))?, message)?.is_some())
}

#[tauri::command(async)]
fn history(root: String, rel: Option<String>) -> Result<Vec<Version>> {
    repo::history(&repo::open_or_init(&PathBuf::from(root))?, rel.as_deref())
}

#[tauri::command(async)]
fn restore(root: String, id: String, rel: Option<String>) -> Result<()> {
    repo::restore(&repo::open_or_init(&PathBuf::from(root))?, &id, rel.as_deref())
}

#[tauri::command(async)]
fn add_files(root: String, dest: String, paths: Vec<String>) -> Result<Vec<String>> {
    let sources: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    repo::add_files(&repo::open_or_init(&PathBuf::from(root))?, &dest, &sources)
}

/// 過去バージョンを一時ファイルに書き出してパスを返す。
#[tauri::command(async)]
fn export_version(root: String, id: String, rel: String) -> Result<String> {
    let path = repo::export_version(&repo::open_or_init(&PathBuf::from(root))?, &id, &rel)?;
    Ok(path.to_string_lossy().to_string())
}

/// ドラッグ取り出し時に表示するアイコン(プラグインが画像ファイルのパスを要求する)。
#[tauri::command(async)]
fn drag_icon() -> Result<String> {
    let path = std::env::temp_dir().join("giter-drag-icon.png");
    std::fs::write(&path, include_bytes!("../icons/32x32.png"))?;
    Ok(path.to_string_lossy().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_drag::init())
        .invoke_handler(tauri::generate_handler![
            open_folder,
            list_dir,
            pending_count,
            save_now,
            history,
            restore,
            add_files,
            export_version,
            drag_icon,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
