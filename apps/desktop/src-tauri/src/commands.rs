//! 界面命令。每个命令只做参数转换，业务逻辑全部在 lfvm-core 中。
//!
//! - 命令一律为 async，核心调用放到阻塞线程池执行，避免文件操作卡住界面（LFVM-P-10）；
//! - 前端不能传入任意文件系统路径：文件夹由 Rust 端弹出系统对话框或接收拖放，
//!   前端只拿到一次性的令牌（LFVM-IF-03、IF-05）；项目内的位置一律用相对路径并由核心校验。

use std::path::PathBuf;
use std::sync::Arc;

use lfvm_core::changes::ChangeSet;
use lfvm_core::compare::{CompareResult, FileDiff};
use lfvm_core::content::{FilePreview, SourceRef};
use lfvm_core::exclude::ExclusionRule;
use lfvm_core::history::{HistoryEntry, TimeMap};
use lfvm_core::ops::restore::{ImpactPlan, WorkspaceOpRequest};
use lfvm_core::ops::single::{FileRestoreCheck, FileRestoreRequest, FileTarget};
use lfvm_core::ops::{OperationDetail, OperationResult, OperationSummary};
use lfvm_core::output::{OutputKind, OutputResult, OutputSource, OutputTarget};
use lfvm_core::project::{AddCheck, DirChild, ProjectOverview, ProjectSummary, RuleView};
use lfvm_core::scheme::{SchemeInfo, SchemeList, SwitchCheck, SwitchRequest, SwitchTarget};
use lfvm_core::search::{SearchPage, SearchQuery};
use lfvm_core::storage::{ClearRequest, ClearResult, StorageReport};
use lfvm_core::trail::Trail;
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
    let result = tauri::async_runtime::spawn_blocking(move || f(&core))
        .await
        .map_err(|e| CoreError::new(ErrorCode::Internal, format!("后台任务异常结束（{e}）")))
        .and_then(|r| r);
    // 用户取消、需要确认等属于正常交互，不记入错误日志
    if let Err(e) = &result
        && !matches!(e.code, ErrorCode::Cancelled | ErrorCode::NeedsConfirmation | ErrorCode::NothingToSave)
    {
        state.core.log_error(std::any::type_name::<F>(), e);
    }
    result
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
    state
        .picked
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(token)
        .cloned()
        .ok_or_else(|| CoreError::new(ErrorCode::InvalidInput, "所选文件夹已失效，请重新选择"))
}

#[tauri::command]
#[specta::specta]
pub async fn list_projects(state: State<'_, AppState>) -> Result<Vec<ProjectSummary>, CoreError> {
    blocking(&state, |core| core.list_projects()).await
}

/// 选择文件夹的用途，决定对话框标题。
#[derive(Debug, Clone, Copy, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum FolderPurpose {
    AddProject,
    SaveTo,
    ExpandTo,
    ExportTo,
}

/// 弹出系统的“选择文件夹”对话框。用户取消时返回 null。
#[tauri::command]
#[specta::specta]
pub async fn pick_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    purpose: FolderPurpose,
) -> Result<Option<PickedFolder>, CoreError> {
    let title = match purpose {
        FolderPurpose::AddProject => "选择要管理的文件夹",
        FolderPurpose::SaveTo => "选择另存到的文件夹",
        FolderPurpose::ExpandTo => "选择展开到的位置",
        FolderPurpose::ExportTo => "选择导出位置",
    };
    let picked =
        tauri::async_runtime::spawn_blocking(move || app.dialog().file().set_title(title).blocking_pick_folder())
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

// ───────────────────────── 时间地图、历史文件、比较 ─────────────────────────

#[tauri::command]
#[specta::specta]
pub async fn time_map(state: State<'_, AppState>, project_id: String) -> Result<TimeMap, CoreError> {
    blocking(&state, move |core| core.time_map(&project_id)).await
}

/// 版本或安全备份中的全部文件与文件夹。
#[tauri::command]
#[specta::specta]
pub async fn source_files(
    state: State<'_, AppState>,
    project_id: String,
    source: SourceRef,
) -> Result<Vec<HistoryEntry>, CoreError> {
    blocking(&state, move |core| core.source_files(&project_id, &source)).await
}

#[tauri::command]
#[specta::specta]
pub async fn preview_file(
    state: State<'_, AppState>,
    project_id: String,
    source: SourceRef,
    path: String,
) -> Result<FilePreview, CoreError> {
    blocking(&state, move |core| core.preview_file(&project_id, &source, &path)).await
}

#[tauri::command]
#[specta::specta]
pub async fn compare_versions(
    state: State<'_, AppState>,
    project_id: String,
    version_a: String,
    version_b: String,
    scope: Option<String>,
) -> Result<CompareResult, CoreError> {
    blocking(&state, move |core| core.compare_versions(&project_id, &version_a, &version_b, scope.as_deref())).await
}

#[tauri::command]
#[specta::specta]
pub async fn diff_file(
    state: State<'_, AppState>,
    project_id: String,
    version_a: String,
    version_b: String,
    path: String,
) -> Result<FileDiff, CoreError> {
    blocking(&state, move |core| core.diff_file(&project_id, &version_a, &version_b, &path)).await
}

// ───────────────────────── 恢复、安全备份、操作记录 ─────────────────────────

/// 单文件恢复的目标：原位置，或另存到用户刚选择的文件夹（令牌来自 pick_folder）。
#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileTargetArg {
    Original,
    SaveAs { token: String },
}

fn file_target(state: &AppState, arg: &FileTargetArg) -> CoreResult<FileTarget> {
    Ok(match arg {
        FileTargetArg::Original => FileTarget::Original,
        FileTargetArg::SaveAs { token } => FileTarget::SaveAs(picked_path(state, token)?),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn check_file_restore(
    state: State<'_, AppState>,
    project_id: String,
    source: SourceRef,
    path: String,
    target: FileTargetArg,
) -> Result<FileRestoreCheck, CoreError> {
    let target = file_target(&state, &target)?;
    blocking(&state, move |core| core.check_file_restore(&project_id, &source, &path, &target)).await
}

#[tauri::command]
#[specta::specta]
pub async fn restore_file(
    state: State<'_, AppState>,
    request: FileRestoreRequest,
    target: FileTargetArg,
    task_id: String,
    on_progress: Channel<ProgressEvent>,
) -> Result<OperationResult, CoreError> {
    let target = file_target(&state, &target)?;
    let task = state.tasks.start(&task_id);
    let progress = ChannelProgress::new(on_progress, task.flag.clone());
    blocking(&state, move |core| core.restore_file(&request, &target, &progress)).await
}

#[tauri::command]
#[specta::specta]
pub async fn plan_restore(
    state: State<'_, AppState>,
    project_id: String,
    version_id: String,
    task_id: String,
    on_progress: Channel<ProgressEvent>,
) -> Result<ImpactPlan, CoreError> {
    let task = state.tasks.start(&task_id);
    let progress = ChannelProgress::new(on_progress, task.flag.clone());
    blocking(&state, move |core| core.plan_restore(&project_id, &version_id, &progress)).await
}

#[tauri::command]
#[specta::specta]
pub async fn restore_version(
    state: State<'_, AppState>,
    request: WorkspaceOpRequest,
    task_id: String,
    on_progress: Channel<ProgressEvent>,
) -> Result<OperationResult, CoreError> {
    let task = state.tasks.start(&task_id);
    let progress = ChannelProgress::new(on_progress, task.flag.clone());
    blocking(&state, move |core| core.restore_version(&request, &progress)).await
}

#[tauri::command]
#[specta::specta]
pub async fn plan_retry(
    state: State<'_, AppState>,
    project_id: String,
    operation_id: String,
    task_id: String,
    on_progress: Channel<ProgressEvent>,
) -> Result<ImpactPlan, CoreError> {
    let task = state.tasks.start(&task_id);
    let progress = ChannelProgress::new(on_progress, task.flag.clone());
    blocking(&state, move |core| core.plan_retry(&project_id, &operation_id, &progress)).await
}

#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn retry_operation(
    state: State<'_, AppState>,
    project_id: String,
    request_id: String,
    operation_id: String,
    fingerprint: String,
    task_id: String,
    on_progress: Channel<ProgressEvent>,
) -> Result<OperationResult, CoreError> {
    let task = state.tasks.start(&task_id);
    let progress = ChannelProgress::new(on_progress, task.flag.clone());
    blocking(&state, move |core| core.retry_operation(&project_id, &request_id, &operation_id, &fingerprint, &progress))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn resolve_incomplete(
    state: State<'_, AppState>,
    project_id: String,
    operation_id: String,
) -> Result<(), CoreError> {
    blocking(&state, move |core| core.resolve_incomplete(&project_id, &operation_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_operations(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Vec<OperationSummary>, CoreError> {
    blocking(&state, move |core| core.list_operations(&project_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn operation_detail(
    state: State<'_, AppState>,
    project_id: String,
    operation_id: String,
) -> Result<OperationDetail, CoreError> {
    blocking(&state, move |core| core.operation_detail(&project_id, &operation_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn open_incomplete(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Option<OperationSummary>, CoreError> {
    blocking(&state, move |core| core.open_incomplete(&project_id)).await
}

// ───────────────────────── 方案 ─────────────────────────

#[tauri::command]
#[specta::specta]
pub async fn list_schemes(state: State<'_, AppState>, project_id: String) -> Result<SchemeList, CoreError> {
    blocking(&state, move |core| core.list_schemes(&project_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn create_scheme(
    state: State<'_, AppState>,
    project_id: String,
    version_id: String,
    name: String,
) -> Result<SchemeInfo, CoreError> {
    blocking(&state, move |core| core.create_scheme(&project_id, &version_id, &name)).await
}

#[tauri::command]
#[specta::specta]
pub async fn rename_scheme(
    state: State<'_, AppState>,
    project_id: String,
    scheme_id: String,
    name: String,
) -> Result<(), CoreError> {
    blocking(&state, move |core| core.rename_scheme(&project_id, &scheme_id, &name)).await
}

#[tauri::command]
#[specta::specta]
pub async fn check_switch(
    state: State<'_, AppState>,
    project_id: String,
    target: SwitchTarget,
    task_id: String,
    on_progress: Channel<ProgressEvent>,
) -> Result<SwitchCheck, CoreError> {
    let task = state.tasks.start(&task_id);
    let progress = ChannelProgress::new(on_progress, task.flag.clone());
    blocking(&state, move |core| core.check_switch(&project_id, &target, &progress)).await
}

#[tauri::command]
#[specta::specta]
pub async fn switch_scheme(
    state: State<'_, AppState>,
    request: SwitchRequest,
    task_id: String,
    on_progress: Channel<ProgressEvent>,
) -> Result<OperationResult, CoreError> {
    let task = state.tasks.start(&task_id);
    let progress = ChannelProgress::new(on_progress, task.flag.clone());
    blocking(&state, move |core| core.switch_scheme(&request, &progress)).await
}

// ───────────────────────── 找回文件 ─────────────────────────

#[tauri::command]
#[specta::specta]
pub async fn search_files(
    state: State<'_, AppState>,
    project_id: String,
    query: SearchQuery,
) -> Result<SearchPage, CoreError> {
    blocking(&state, move |core| core.search_files(&project_id, &query)).await
}

/// 确保缩略图已生成；失败时返回原因。成功后界面经预览协议加载 `t.<project_id>.<hash>`。
#[tauri::command]
#[specta::specta]
pub async fn ensure_thumbnail(state: State<'_, AppState>, project_id: String, hash: String) -> Result<(), CoreError> {
    blocking(&state, move |core| core.ensure_thumbnail(&project_id, &hash)).await
}

#[tauri::command]
#[specta::specta]
pub async fn file_trail(
    state: State<'_, AppState>,
    project_id: String,
    path: String,
    route: SwitchTarget,
) -> Result<Trail, CoreError> {
    blocking(&state, move |core| core.file_trail(&project_id, &path, &route)).await
}

// ───────────────────────── 展开、导出、存储 ─────────────────────────

#[tauri::command]
#[specta::specta]
pub async fn suggest_output_name(
    state: State<'_, AppState>,
    project_id: String,
    source: OutputSource,
) -> Result<String, CoreError> {
    blocking(&state, move |core| core.suggest_output_name(&project_id, &source)).await
}

/// 展开或导出。`location_token` 来自 pick_folder；输出到该位置下名为 `folder_name` 的新文件夹。
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn output_version(
    state: State<'_, AppState>,
    project_id: String,
    request_id: String,
    kind: OutputKind,
    source: OutputSource,
    location_token: String,
    folder_name: String,
    task_id: String,
    on_progress: Channel<ProgressEvent>,
) -> Result<OutputResult, CoreError> {
    let parent = picked_path(&state, &location_token)?;
    let task = state.tasks.start(&task_id);
    let progress = ChannelProgress::new(on_progress, task.flag.clone());
    blocking(&state, move |core| {
        core.output_version(&project_id, &request_id, kind, &source, &OutputTarget { parent, folder_name }, &progress)
    })
    .await
}

/// 在系统文件管理器中打开展开或导出的文件夹。
#[tauri::command]
#[specta::specta]
pub async fn reveal_output(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: String,
    operation_id: String,
) -> Result<(), CoreError> {
    let path = blocking(&state, move |core| core.output_path(&project_id, &operation_id)).await?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| CoreError::new(ErrorCode::Io, format!("无法打开文件夹（{e}）")))
}

#[tauri::command]
#[specta::specta]
pub async fn storage_report(state: State<'_, AppState>, project_id: String) -> Result<StorageReport, CoreError> {
    blocking(&state, move |core| core.storage_report(&project_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn clear_storage(
    state: State<'_, AppState>,
    project_id: String,
    request: ClearRequest,
) -> Result<ClearResult, CoreError> {
    blocking(&state, move |core| core.clear_storage(&project_id, &request)).await
}
