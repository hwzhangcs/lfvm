//! 单文件写入协议（SRS 3.3.2.4 第 3～5 步）：恢复单个历史文件、从安全备份找回、从搜索结果恢复共用。
//!
//! 1. 目标已有文件时，界面先显示其路径和修改时间，由用户确认替换；
//! 2. 历史内容先写入同目录临时文件并校验；
//! 3. 目标已有文件时，先为它创建安全备份（原因“恢复文件××前”）；
//! 4. 再次确认目标在用户确认后未被修改，然后原子替换，并校验写入结果。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use lfvm_platform::EntryKind;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::content::{SourceRef, find_file};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::exclude::{self, Matcher};
use crate::hash::hash_file;
use crate::model::EntryType;
use crate::ops::apply::{check_no_links, prepare_temp, verify_unchanged};
use crate::ops::backup::BackupRequest;
use crate::ops::plan::{Action, PlanItem};
use crate::ops::restore::TargetRef;
use crate::ops::{
    ItemState, NewOp, OpStatus, OpType, OperationResult, Phase, failpoint, finish_op, insert_items, insert_op,
    result_of, set_item_state, set_phase,
};
use crate::paths::{self, RelPath, path_key};
use crate::progress::Progress;
use crate::project::load_project;
use crate::store::ObjectStore;
use crate::Core;

/// 恢复到哪里。另存的文件夹由用户通过系统对话框选择（LFVM-IF-03）。
#[derive(Debug, Clone)]
pub enum FileTarget {
    /// 恢复到原位置（版本中的路径相对于项目文件夹；备份中的路径相对于备份的根目录）。
    Original,
    /// 另存到这个文件夹，文件名不变。
    SaveAs(PathBuf),
}

/// 恢复前的检查结果，供界面显示确认信息。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct FileRestoreCheck {
    pub target_path: String,
    /// 目标位置已有同名文件（需要用户确认替换）。
    pub exists: bool,
    #[cfg_attr(feature = "specta", specta(type = Option<specta_typescript::Number>))]
    pub existing_size: Option<i64>,
    #[cfg_attr(feature = "specta", specta(type = Option<specta_typescript::Number>))]
    pub existing_modified_at: Option<i64>,
    /// 确认替换时交回；替换前核对，若不一致说明目标在确认后被修改。
    pub confirm_token: Option<String>,
    /// 原位置属于当前排除项：不允许原位覆盖，只能另存（规则 R-05）。
    pub excluded: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct FileRestoreRequest {
    pub project_id: String,
    pub request_id: String,
    pub source: SourceRef,
    pub path: String,
    /// 目标已有文件时必须给出（来自 [`FileRestoreCheck::confirm_token`]）。
    pub confirm_token: Option<String>,
}

struct Resolved {
    root: PathBuf,
    rel: RelPath,
    external_root: Option<PathBuf>,
    hash: String,
    size: u64,
    excluded: bool,
}

fn token_of(meta: &std::fs::Metadata) -> String {
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
    format!("{}:{mtime}", meta.len())
}

impl Core {
    fn resolve_file_target(&self, project_id: &str, source: &SourceRef, path: &str, target: &FileTarget) -> CoreResult<Resolved> {
        let db = self.db();
        let project = load_project(&db, project_id)?;
        let rec = find_file(&db, project_id, source, path)?;
        let (EntryType::File, Some(hash)) = (rec.entry_type, rec.hash.clone()) else {
            return Err(CoreError::new(ErrorCode::InvalidInput, "只能恢复文件，不能恢复文件夹"));
        };
        let store = ObjectStore::new(&self.project_store_dir(project_id));
        if !store.contains(&hash) {
            return Err(CoreError::new(ErrorCode::ContentCorrupted, "这个文件的历史内容缺失，无法恢复").with_path(path));
        }
        let backup_root: Option<PathBuf> = match source {
            SourceRef::Backup { backup_id } => db
                .query_row("SELECT external_root FROM safety_backups WHERE backup_id = ?1", [backup_id], |r| {
                    r.get::<_, Option<String>>(0)
                })
                .optional()?
                .flatten()
                .map(PathBuf::from),
            SourceRef::Version { .. } => None,
        };
        let rules = exclude::load_rules(&db, project_id)?;
        drop(db);

        let (root, rel, external_root) = match target {
            FileTarget::Original => match backup_root {
                Some(ext) => (ext.clone(), RelPath::parse(&rec.rel)?, Some(ext)),
                None => (project.root.clone(), RelPath::parse(&rec.rel)?, None),
            },
            FileTarget::SaveAs(dir) => {
                let meta = std::fs::symlink_metadata(dir).map_err(|e| CoreError::io(e, dir))?;
                if lfvm_platform::classify(&meta) != EntryKind::Dir {
                    return Err(CoreError::new(ErrorCode::InvalidInput, "请选择一个文件夹").with_path(dir));
                }
                let dir = paths::canonical(dir)?;
                if paths::same_or_nested(&dir, self.data_dir(), lfvm_platform::default_case_policy()) {
                    return Err(CoreError::new(ErrorCode::DirectoryOverlap, "不能保存到本软件的历史存储位置").with_path(&dir));
                }
                let name = RelPath::parse(&rec.rel)?.file_name().to_owned();
                (dir.clone(), RelPath::parse(&name)?, Some(dir))
            }
        };
        let excluded = external_root.is_none()
            && Matcher::new(&rules, project.policy)?.is_excluded(&rel, false);
        Ok(Resolved { root, rel, external_root, hash, size: rec.size.unwrap_or(0), excluded })
    }

    /// 恢复前检查目标位置（SRS 3.3.2.4 第 3 步）。
    pub fn check_file_restore(
        &self,
        project_id: &str,
        source: &SourceRef,
        path: &str,
        target: &FileTarget,
    ) -> CoreResult<FileRestoreCheck> {
        let r = self.resolve_file_target(project_id, source, path, target)?;
        let abs = r.rel.to_path(&r.root);
        let meta = std::fs::symlink_metadata(&abs).ok();
        if meta.as_ref().is_some_and(|m| lfvm_platform::classify(m) != EntryKind::File) {
            return Err(CoreError::new(ErrorCode::StructureConflict, "目标位置是一个文件夹或快捷链接，无法用文件替换").with_path(&abs));
        }
        Ok(FileRestoreCheck {
            target_path: abs.to_string_lossy().into_owned(),
            exists: meta.is_some(),
            existing_size: meta.as_ref().map(|m| m.len() as i64),
            existing_modified_at: meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64),
            confirm_token: meta.as_ref().map(token_of),
            excluded: r.excluded,
        })
    }

    /// 按单文件写入协议恢复一个历史文件。项目处于“未完成”状态时也允许（SRS 6.1.1）。
    pub fn restore_file(&self, req: &FileRestoreRequest, target: &FileTarget, progress: &dyn Progress) -> CoreResult<OperationResult> {
        let _guard = self.begin_write(&req.project_id)?;
        let r = self.resolve_file_target(&req.project_id, &req.source, &req.path, target)?;
        if r.excluded {
            return Err(CoreError::new(ErrorCode::InvalidInput, "原位置属于当前排除项，不能原位覆盖，请选择“另存到其他位置”"));
        }
        let abs = r.rel.to_path(&r.root);
        check_no_links(&r.root, &r.rel, &mut HashSet::new())?;
        let existing = std::fs::symlink_metadata(&abs).ok();
        if let Some(m) = &existing {
            if lfvm_platform::classify(m) != EntryKind::File {
                return Err(CoreError::new(ErrorCode::StructureConflict, "目标位置是一个文件夹或快捷链接，无法用文件替换").with_path(&abs));
            }
            match &req.confirm_token {
                None => {
                    return Err(CoreError::new(ErrorCode::NeedsConfirmation, "目标位置已有同名文件，需要确认是否替换").with_path(&abs));
                }
                Some(t) if *t != token_of(m) => {
                    return Err(CoreError::new(ErrorCode::ChangedExternally, "目标文件在确认后被修改，为避免覆盖新的修改，没有替换").with_path(&abs));
                }
                _ => {}
            }
        }

        let label = format!("文件 {}", r.rel.file_name());
        let op_id = {
            let db = self.db();
            match insert_op(
                &db,
                &NewOp {
                    project_id: &req.project_id,
                    request_id: &req.request_id,
                    op_type: OpType::Restore,
                    target_ref: TargetRef { label: label.clone(), scheme: None, path: None }.to_json(),
                    resolved_version_id: match &req.source {
                        SourceRef::Version { version_id } => Some(version_id.as_str()),
                        SourceRef::Backup { .. } => None,
                    },
                    retry_of: None,
                },
            )? {
                Ok(id) => id,
                Err(existing) => return result_of(&db, &existing),
            }
        };
        let finish = |status: OpStatus, msg: &str| -> CoreResult<OperationResult> {
            let db = self.db();
            finish_op(&db, &op_id, status, msg)?;
            result_of(&db, &op_id)
        };

        let store = ObjectStore::new(&self.project_store_dir(&req.project_id));
        // 已有文件的当前摘要（备份与替换前核对都以它为准）
        let before = match &existing {
            Some(_) => match hash_file(&abs) {
                Ok((h, size)) => Some((h, size)),
                Err(e) => return finish(OpStatus::Failed, &CoreError::io(e, &abs).message),
            },
            None => None,
        };
        let item = PlanItem {
            path: r.rel.to_string(),
            key: path_key(r.rel.as_str(), lfvm_platform::CasePolicy::Insensitive),
            action: if before.is_some() { Action::Replace } else { Action::Create },
            entry_type: EntryType::File,
            before_hash: before.as_ref().map(|(h, _)| h.clone()),
            after_hash: Some(r.hash.clone()),
            before_size: before.as_ref().map(|(_, s)| *s as i64),
            after_size: Some(r.size as i64),
            type_change: false,
        };
        insert_items(&self.db(), &op_id, std::slice::from_ref(&item))?;

        // 1. 写入临时文件并校验
        let tmp = match prepare_temp(&store, &r.hash, &abs) {
            Ok(t) => t,
            Err(e) => return finish(OpStatus::Failed, &e.message),
        };
        let cleanup = |tmp: &Path| {
            let _ = std::fs::remove_file(tmp);
        };

        // 2. 为已有文件创建安全备份
        set_phase(&self.db(), &op_id, Phase::Protecting)?;
        if before.is_some() {
            let backup = self.create_backup(
                &BackupRequest {
                    project_id: &req.project_id,
                    operation_id: &op_id,
                    reason: format!("恢复文件{}前", r.rel.file_name()),
                    root: &r.root,
                    external_root: r.external_root.as_deref(),
                    items: std::slice::from_ref(&item),
                },
                progress,
            );
            if let Err(e) = backup {
                cleanup(&tmp);
                return finish(OpStatus::Failed, &e.message);
            }
        }

        // 3. 再次确认目标未被修改，然后替换
        set_phase(&self.db(), &op_id, Phase::Executing)?;
        let _ = set_item_state(&self.db(), &op_id, 0, &item.key, ItemState::Applying, None);
        let replaced = (|| {
            match &before {
                Some((h, _)) => verify_unchanged(&abs, Some(h))?,
                None if std::fs::symlink_metadata(&abs).is_ok() => {
                    return Err(CoreError::new(ErrorCode::ChangedExternally, "目标位置出现了新的同名文件，没有替换").with_path(&abs));
                }
                None => {}
            }
            failpoint::hit("replace")?;
            lfvm_platform::atomic_replace(&tmp, &abs).map_err(|e| CoreError::io(e, &abs))
        })();
        if let Err(e) = replaced {
            cleanup(&tmp);
            let _ = set_item_state(&self.db(), &op_id, 0, &item.key, ItemState::Failed, Some(&e));
            let note = if before.is_some() { "原文件没有被改动，也已存入安全备份" } else { "没有写入任何文件" };
            return finish(OpStatus::Failed, &format!("{}（{note}）", e.message));
        }

        // 4. 校验写入结果
        match hash_file(&abs) {
            Ok((h, _)) if h == r.hash => {
                let _ = set_item_state(&self.db(), &op_id, 0, &item.key, ItemState::Done, None);
                let db = self.db();
                finish_op(&db, &op_id, OpStatus::Succeeded, &format!("已恢复到 {}", abs.to_string_lossy()))?;
                result_of(&db, &op_id)
            }
            _ => {
                let e = CoreError::new(ErrorCode::ContentCorrupted, "写入后校验不一致");
                let _ = set_item_state(&self.db(), &op_id, 0, &item.key, ItemState::Failed, Some(&e));
                finish(OpStatus::Incomplete, "文件已写入但校验不一致，结果无法确认；原文件可从安全备份中找回")
            }
        }
    }
}
