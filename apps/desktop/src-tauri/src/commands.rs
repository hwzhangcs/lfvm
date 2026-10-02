//! 界面命令。每个命令只做参数转换，业务逻辑全部在 lfvm-core 中。
//!
//! - 命令一律为 async，核心调用放到阻塞线程池执行，避免文件操作卡住界面（LFVM-P-10）；
//! - 前端不能传入任意文件系统路径：文件夹由 Rust 端弹出系统对话框或接收拖放，
//!   前端只拿到一次性的令牌（LFVM-IF-03、IF-05）；项目内的位置一律用相对路径并由核心校验。

use std::path::PathBuf;
use std::sync::Arc;

use lfvm_core::changes::ChangeSet;
use lfvm_core::exclude::ExclusionRule;
use lfvm_core::project::{AddCheck, DirChild, ProjectOverview, ProjectSummary, RuleView};
use lfvm_core::version::{SaveRequest, SaveResult};
use lfvm_core::{Core, CoreError, CoreResult, ErrorCode};
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::AppState;
use crate::tasks::{ChannelProgress, ProgressEvent};

async fn blocking<T, F>(state: &AppState, f: F) -> CoreResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Core) -> CoreResult<T> + Send + 'static,
{
    let core: Arc<Core> = state.core.clone();
    tauri::async_runtime::spawn_blocking(move || f(&core))
        .await
        .map_err(|e| CoreError::new(ErrorCode::Internal, format!("后台任务异常结束（{e}）")))?
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct AppInfo {
    pub version: String,
    /// 历史存储目录（界面在“存储管理”中显示）。
    pub data_dir: String,
    /// windows / macos / linux
    pub os: String,
}

#[tauri::command]
#[specta::specta]
pub async fn app_info(state: State<'_, AppState>) -> Result<AppInfo, CoreError> {
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        data_dir: state.core.data_dir().to_string_lossy().into_owned(),
        os: std::env::consts::OS.to_owned(),
    })
}

// ───────────────────────── 项目 ─────────────────────────

/// 用户通过对话框或拖放选中的文件夹。
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct PickedFolder {
    /// 一次性令牌，加入项目时交回。
    pub token: String,
    /// 仅用于显示。
    pub path: String,
}

fn picked_path(state: &AppState, token: &str) -> CoreResult<PathBuf> {
    state.picked.lock().unwrap_or_else(|e| e.into_inner()).get(token).cloned().ok_or_else(|| {
        CoreError::new(ErrorCode::InvalidInput, "所选文件夹已失效，请重新选择")
    })
}

#[tauri::command]
#[specta::specta]
pub async fn list_projects(state: State<'_, AppState>) -> Result<Vec<ProjectSummary>, CoreError> {
    blocking(&state, |core| core.list_projects()).await
}

/// 弹出系统的“选择文件夹”对话框。用户取消时返回 null。
#[tauri::command]
#[specta::specta]
pub async fn pick_folder(app: AppHandle, state: State<'_, AppState>) -> Result<Option<PickedFolder>, CoreError> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().set_title("选择要管理的文件夹").blocking_pick_folder()
    })
    .await
    .map_err(|e| CoreError::new(ErrorCode::Internal, e.to_string()))?;
    let Some(fp) = picked else { return Ok(None) };
    let path = fp.into_path().map_err(|e| CoreError::new(ErrorCode::InvalidPath, e.to_string()))?;
    Ok(Some(state.remember_folder(path)))
}

#[tauri::command]
#[specta::specta]
pub async fn check_add_project(state: State<'_, AppState>, token: String) -> Result<AddCheck, CoreError> {
    let path = picked_path(&state, &token)?;
    blocking(&state, move |core| core.check_new_project(&path)).await
}

/// 加入项目。`associate_previous` 为用户选择关联的原历史项目 ID。
#[tauri::command]
#[specta::specta]
pub async fn add_project(
    state: State<'_, AppState>,
    token: String,
    associate_previous: Option<String>,
) -> Result<ProjectSummary, CoreError> {
    let path = picked_path(&state, &token)?;
    let result = blocking(&state, move |core| core.add_project(&path, associate_previous.as_deref())).await;
    if result.is_ok() {
        state.picked.lock().unwrap_or_else(|e| e.into_inner()).remove(&token);
    }
    result
}

#[tauri::command]
#[specta::specta]
pub async fn remove_project(state: State<'_, AppState>, project_id: String) -> Result<(), CoreError> {
    blocking(&state, move |core| core.remove_project(&project_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn open_project(state: State<'_, AppState>, project_id: String) -> Result<ProjectOverview, CoreError> {
    blocking(&state, move |core| core.open_project(&project_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn project_overview(state: State<'_, AppState>, project_id: String) -> Result<ProjectOverview, CoreError> {
    blocking(&state, move |core| core.overview(&project_id)).await
}

/// 在系统文件管理器中打开项目文件夹。
#[tauri::command]
#[specta::specta]
pub async fn reveal_project_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: String,
) -> Result<(), CoreError> {
    let summary = blocking(&state, move |core| core.overview(&project_id)).await?.project;
    app.opener()
        .open_path(summary.root_path, None::<&str>)
        .map_err(|e| CoreError::new(ErrorCode::Io, format!("无法打开文件夹（{e}）")))
}

// ───────────────────────── 当前变化与保存 ─────────────────────────

#[tauri::command]
#[specta::specta]
pub async fn current_changes(
    state: State<'_, AppState>,
    project_id: String,
    task_id: String,
    on_progress: Channel<ProgressEvent>,
) -> Result<ChangeSet, CoreError> {
    let task = state.tasks.start(&task_id);
    let progress = ChannelProgress::new(on_progress, task.flag.clone());
    blocking(&state, move |core| core.current_changes(&project_id, &progress)).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_version(
    state: State<'_, AppState>,
    request: SaveRequest,
    task_id: String,
    on_progress: Channel<ProgressEvent>,
) -> Result<SaveResult, CoreError> {
    let task = state.tasks.start(&task_id);
    let progress = ChannelProgress::new(on_progress, task.flag.clone());
    blocking(&state, move |core| core.save_version(&request, &progress)).await
}

/// 请求取消长任务。任务会在处理完当前文件后停止。
#[tauri::command]
#[specta::specta]
pub async fn cancel_task(state: State<'_, AppState>, task_id: String) -> Result<bool, CoreError> {
    Ok(state.tasks.cancel(&task_id))
}

// ───────────────────────── 排除项 ─────────────────────────

#[tauri::command]
#[specta::specta]
pub async fn exclusion_rules(state: State<'_, AppState>, project_id: String) -> Result<Vec<RuleView>, CoreError> {
    blocking(&state, move |core| core.exclusion_rules(&project_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn count_rule_matches(
    state: State<'_, AppState>,
    project_id: String,
    rules: Vec<ExclusionRule>,
) -> Result<Vec<u32>, CoreError> {
    blocking(&state, move |core| core.count_rule_matches(&project_id, &rules)).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_exclusion_rules(
    state: State<'_, AppState>,
    project_id: String,
    rules: Vec<ExclusionRule>,
) -> Result<(), CoreError> {
    blocking(&state, move |core| core.save_exclusion_rules(&project_id, &rules)).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_workspace_dir(
    state: State<'_, AppState>,
    project_id: String,
    dir: Option<String>,
) -> Result<Vec<DirChild>, CoreError> {
    blocking(&state, move |core| core.list_workspace_dir(&project_id, dir.as_deref())).await
}
