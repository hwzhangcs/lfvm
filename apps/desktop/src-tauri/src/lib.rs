//! 桌面外壳。只做三件事：初始化核心、把核心功能注册为界面命令、处理窗口与系统对话框。
//! 前端只能调用这里登记的固定命令，不提供任意路径读写或执行命令的接口（LFVM-IF-05）。

mod commands;
mod preview;
mod tasks;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use lfvm_core::Core;
use serde::{Deserialize, Serialize};
use tauri::{DragDropEvent, Manager, WindowEvent};
use tauri_specta::{Builder, ErrorHandlingMode, Event, collect_commands, collect_events};

use crate::commands::PickedFolder;
use crate::tasks::TaskRegistry;

pub struct AppState {
    pub core: Arc<Core>,
    pub tasks: TaskRegistry,
    /// 用户通过对话框或拖放选中的文件夹：令牌 → 路径。
    pub picked: Mutex<HashMap<String, PathBuf>>,
}

impl AppState {
    pub fn remember_folder(&self, path: PathBuf) -> PickedFolder {
        let token = lfvm_core::new_id();
        let display = path.to_string_lossy().into_owned();
        self.picked.lock().unwrap_or_else(|e| e.into_inner()).insert(token.clone(), path);
        PickedFolder { token, path: display }
    }
}

/// 用户把文件或文件夹拖入窗口。拖入的不是文件夹时 `folder` 为 null。
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Event)]
pub struct FolderDropped {
    pub folder: Option<PickedFolder>,
}

/// 所有界面命令都在这里登记；同时用于生成前端的 `bindings.ts`。
pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .error_handling(ErrorHandlingMode::Throw)
        .commands(collect_commands![
            commands::app_info,
            commands::list_projects,
            commands::pick_folder,
            commands::check_add_project,
            commands::add_project,
            commands::remove_project,
            commands::open_project,
            commands::project_overview,
            commands::reveal_project_folder,
            commands::current_changes,
            commands::save_version,
            commands::cancel_task,
            commands::exclusion_rules,
            commands::count_rule_matches,
            commands::save_exclusion_rules,
            commands::list_workspace_dir,
            commands::time_map,
            commands::source_files,
            commands::preview_file,
            commands::compare_versions,
            commands::diff_file,
        ])
        .events(collect_events![FolderDropped])
}

pub const BINDINGS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/bindings.ts");

pub fn export_bindings(builder: &Builder<tauri::Wry>) {
    builder
        .export(
            specta_typescript::Typescript::default()
                .header("// 此文件由 src-tauri 自动生成（tauri-specta），请勿手动修改。\n"),
            BINDINGS_PATH,
        )
        .expect("生成 bindings.ts 失败");
}

fn on_drag_drop(window: &tauri::Window, event: &DragDropEvent) {
    let DragDropEvent::Drop { paths, .. } = event else { return };
    let state = window.state::<AppState>();
    let folder = paths.iter().find(|p| p.is_dir()).map(|p| state.remember_folder(p.clone()));
    let _ = FolderDropped { folder }.emit(window.app_handle());
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = specta_builder();
    #[cfg(debug_assertions)]
    export_bindings(&builder);

    tauri::Builder::default()
        // 必须最先注册：第二个程序实例启动时，转而激活已有窗口（LFVM-AT-07）。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .register_asynchronous_uri_scheme_protocol(preview::SCHEME, preview::handle)
        .invoke_handler(builder.invoke_handler())
        .on_window_event(|window, event| {
            if let WindowEvent::DragDrop(e) = event {
                on_drag_drop(window, e);
            }
        })
        .setup(move |app| {
            builder.mount_events(app);
            // 单独的子目录：WebView 也会在应用数据目录下写缓存，不能计入历史占用（SRS 5.1.0）。
            let data_dir = app.path().app_local_data_dir()?.join("history");
            let core = Core::open(data_dir)?;
            app.manage(AppState {
                core: Arc::new(core),
                tasks: TaskRegistry::default(),
                picked: Mutex::new(HashMap::new()),
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("启动程序失败");
}

#[cfg(test)]
mod tests {
    /// `cargo test -p lfvm-desktop` 即可更新前端类型，无需启动界面。
    #[test]
    fn export_bindings() {
        super::export_bindings(&super::specta_builder());
        assert!(std::path::Path::new(super::BINDINGS_PATH).is_file());
    }
}
