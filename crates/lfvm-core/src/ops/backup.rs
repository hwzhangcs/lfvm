//! 操作前安全备份（SRS 3.3.3.1）。
//!
//! 只备份将被替换或删除的现存文件（不是整个项目），逐个复制并校验大小和摘要，全部一致才置为“就绪”。
//! 复制、校验、权限或空间检查任一步失败，备份标为“失败”，原操作中止，工作区保持不变。

use std::path::Path;

use rusqlite::params;

use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::hash::hash_file;
use crate::model::EntryType;
use crate::ops::failpoint;
use crate::ops::plan::{Action, PlanItem};
use crate::paths::{RelPath, path_key};
use crate::progress::{Progress, Stage};
use crate::store::ObjectStore;
use crate::{Core, new_id, now_ms};

/// 留给数据库和日志的余量。
const SPACE_MARGIN: u64 = 64 * 1024 * 1024;

/// 检查 `dir` 所在磁盘是否还有 `need` 字节可用（LFVM-AT-04：分别检查历史存储盘和目标盘）。
pub(crate) fn ensure_space(dir: &Path, need: u64, what: &str) -> CoreResult<()> {
    failpoint::hit("space")?;
    if need == 0 {
        return Ok(());
    }
    let avail = fs4::available_space(dir).map_err(|e| CoreError::io(e, dir))?;
    if avail < need.saturating_add(SPACE_MARGIN) {
        return Err(CoreError::new(
            ErrorCode::NoSpace,
            format!("{what}所在磁盘空间不足：需要约 {} MB，可用 {} MB", need.div_ceil(1 << 20), avail >> 20),
        )
        .with_path(dir));
    }
    Ok(())
}

pub(crate) struct BackupRequest<'a> {
    pub project_id: &'a str,
    pub operation_id: &'a str,
    pub reason: String,
    /// 文件所在的根目录（项目文件夹，或另存时用户选择的文件夹）。
    pub root: &'a Path,
    /// 另存覆盖项目外文件时为 Some(root)。
    pub external_root: Option<&'a Path>,
    pub items: &'a [PlanItem],
}

impl Core {
    /// 创建安全备份。没有现存文件会被替换或删除时返回 Ok(None)（“无需备份”）。
    pub(crate) fn create_backup(&self, req: &BackupRequest<'_>, progress: &dyn Progress) -> CoreResult<Option<String>> {
        let files: Vec<&PlanItem> = req
            .items
            .iter()
            .filter(|i| matches!(i.action, Action::Replace | Action::Delete) && i.entry_type == EntryType::File)
            .collect();
        let dirs: Vec<&PlanItem> =
            req.items.iter().filter(|i| i.action == Action::Delete && i.entry_type == EntryType::Directory).collect();
        if files.is_empty() && dirs.is_empty() {
            return Ok(None);
        }
        let store = ObjectStore::new(&self.project_store_dir(req.project_id));
        store.ensure_dirs()?;
        let need: u64 = files
            .iter()
            .filter(|i| i.before_hash.as_deref().is_none_or(|h| !store.contains(h)))
            .map(|i| i.before_size.unwrap_or(0) as u64)
            .sum();
        ensure_space(self.data_dir(), need, "历史存储位置")?;

        let backup_id = new_id();
        self.db().execute(
            "INSERT INTO safety_backups (backup_id, project_id, operation_id, reason, created_at, status, expected_file_count, external_root)
             VALUES (?1, ?2, ?3, ?4, ?5, 'creating', ?6, ?7)",
            params![
                backup_id,
                req.project_id,
                req.operation_id,
                req.reason.chars().take(200).collect::<String>(),
                now_ms(),
                files.len() as i64,
                req.external_root.map(|p| p.to_string_lossy().into_owned()),
            ],
        )?;

        let mut created = Vec::new();
        let result = self.fill_backup(req, &backup_id, &files, &dirs, &store, &mut created, progress);
        if let Err(e) = &result {
            let _ = self.db().execute("UPDATE safety_backups SET status = 'failed' WHERE backup_id = ?1", [&backup_id]);
            let db = self.db();
            for h in &created {
                let known: bool = db
                    .query_row(
                        "SELECT 1 FROM content_objects WHERE project_id = ?1 AND content_hash = ?2",
                        params![req.project_id, h],
                        |_| Ok(()),
                    )
                    .is_ok();
                if !known {
                    let _ = store.remove(h);
                }
            }
            return Err(CoreError {
                code: e.code,
                message: format!("安全备份没有创建成功，操作已停止，文件没有被改动。原因：{}", e.message),
                path: e.path.clone(),
            });
        }
        Ok(Some(backup_id))
    }

    #[allow(clippy::too_many_arguments)]
    fn fill_backup(
        &self,
        req: &BackupRequest<'_>,
        backup_id: &str,
        files: &[&PlanItem],
        dirs: &[&PlanItem],
        store: &ObjectStore,
        created: &mut Vec<String>,
        progress: &dyn Progress,
    ) -> CoreResult<()> {
        let total: u64 = files.iter().map(|i| i.before_size.unwrap_or(0) as u64).sum();
        let mut done = 0u64;
        progress.report(Stage::BackingUp, 0, total);
        // 1. 逐个复制；复制时计算的摘要与计划不一致，说明文件在确认后被改过
        for it in files {
            failpoint::hit("backup_copy")?;
            let src = RelPath::parse(&it.path)?.to_path(req.root);
            let got = store.ingest(&src, it.before_hash.as_deref())?;
            if got.created {
                created.push(got.hash.clone());
            }
            done += got.size;
            progress.report(Stage::BackingUp, done, total);
        }
        // 2. 逐项校验备份内容的大小和摘要
        progress.report(Stage::Verifying, 0, files.len() as u64);
        for (n, it) in files.iter().enumerate() {
            failpoint::hit("backup_verify")?;
            let h = it.before_hash.as_deref().unwrap_or_default();
            let obj = store.object_path(h)?;
            let (actual, size) = hash_file(&obj).map_err(|e| CoreError::io(e, &obj))?;
            if actual != h || Some(size as i64) != it.before_size {
                return Err(CoreError::new(ErrorCode::ContentCorrupted, "备份内容校验不一致").with_path(&it.path));
            }
            progress.report(Stage::Verifying, n as u64 + 1, files.len() as u64);
        }
        // 3. 登记并置为就绪
        let policy = lfvm_platform::CasePolicy::Insensitive;
        let mut db = self.db();
        let tx = db.transaction()?;
        {
            let mut ins = tx.prepare(
                "INSERT OR IGNORE INTO backup_files (backup_id, path_key, relative_path, name_key, ext_key, entry_type, size, content_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            let mut obj = tx.prepare(
                "INSERT INTO content_objects (project_id, content_hash, byte_size, state, ref_count) VALUES (?1, ?2, ?3, 'ready', 1)
                 ON CONFLICT (project_id, content_hash) DO UPDATE SET ref_count = ref_count + 1, state = 'ready'",
            )?;
            let mut link = tx.prepare(
                "UPDATE operation_items SET backup_file_ref = ?3 WHERE operation_id = ?1 AND relative_path = ?2 AND action != 'keep'",
            )?;
            for it in files.iter().chain(dirs.iter()) {
                let rel = RelPath::parse(&it.path)?;
                ins.execute(params![
                    backup_id,
                    it.key,
                    it.path,
                    path_key(rel.file_name(), policy),
                    rel.extension().map(|x| path_key(x, policy)).unwrap_or_default(),
                    it.entry_type.as_str(),
                    it.before_size,
                    it.before_hash,
                ])?;
                if let Some(h) = &it.before_hash {
                    obj.execute(params![req.project_id, h, it.before_size.unwrap_or(0)])?;
                }
                link.execute(params![req.operation_id, it.path, backup_id])?;
            }
        }
        tx.execute("UPDATE safety_backups SET status = 'ready' WHERE backup_id = ?1", [backup_id])?;
        tx.commit()?;
        Ok(())
    }
}
