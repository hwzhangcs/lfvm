//! 会改写工作区的操作：整版恢复、切换方案、单文件恢复（SRS 3.3.2.4、3.3.2.5、3.3.3、3.3.4.2、6.1.1）。
//!
//! 普通文件夹没有事务能力（规则 R-06），因此：
//! - 写入前先保存操作计划和逐项前状态，并为将被覆盖或删除的文件创建安全备份；备份未就绪不写入；
//! - 逐项写入，每项写入前确认文件在备份后未被外部修改，写入后校验；
//! - 任一项失败立即停止，记为“未完成”，保留安全备份，提供找回和重试入口；
//! - 程序异常退出后，重新打开项目时核对实际文件状态，统一记为“未完成”，不在后台自动继续写入。

pub(crate) mod apply;
pub(crate) mod backup;
pub(crate) mod plan;
pub mod restore;
pub mod single;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::hash::hash_file;
use crate::paths::RelPath;
use crate::project::load_project;
use crate::{Core, new_id, now_ms};

pub use plan::{Action, PlanItem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum OpType {
    Save,
    Restore,
    Switch,
    Expand,
    Export,
    Clear,
}

impl OpType {
    pub fn as_str(self) -> &'static str {
        match self {
            OpType::Save => "save",
            OpType::Restore => "restore",
            OpType::Switch => "switch",
            OpType::Expand => "expand",
            OpType::Export => "export",
            OpType::Clear => "clear",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "save" => OpType::Save,
            "switch" => OpType::Switch,
            "expand" => OpType::Expand,
            "export" => OpType::Export,
            "clear" => OpType::Clear,
            _ => OpType::Restore,
        }
    }
}

/// 操作状态（SRS 6.1.1）：“失败”表示工作区尚未被改动；只要已有文件被改动或无法确定，就是“未完成”。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum OpStatus {
    Running,
    Succeeded,
    Cancelled,
    Failed,
    Incomplete,
}

impl OpStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            OpStatus::Running => "running",
            OpStatus::Succeeded => "succeeded",
            OpStatus::Cancelled => "cancelled",
            OpStatus::Failed => "failed",
            OpStatus::Incomplete => "incomplete",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "succeeded" => OpStatus::Succeeded,
            "cancelled" => OpStatus::Cancelled,
            "failed" => OpStatus::Failed,
            "incomplete" => OpStatus::Incomplete,
            _ => OpStatus::Running,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Protecting,
    Executing,
    Committing,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Phase::Protecting => "protecting",
            Phase::Executing => "executing",
            Phase::Committing => "committing",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ItemFailure {
    pub path: String,
    pub message: String,
}

/// 一次写操作的结果（表 4-11 OperationResult）。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct OperationResult {
    pub operation_id: String,
    pub status: OpStatus,
    pub message: String,
    /// 已完成、失败、未处理的路径。
    pub completed: Vec<String>,
    pub failed: Vec<ItemFailure>,
    pub pending: Vec<String>,
    /// 本次创建的安全备份；没有现存文件被覆盖或删除时为 null（“无需备份”）。
    pub backup_id: Option<String>,
}

/// 操作记录列表中的一项（SRS 3.3.3.2）。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct OperationSummary {
    pub operation_id: String,
    pub op_type: OpType,
    pub status: OpStatus,
    /// 显示用的操作对象，如“版本 V5”“方案“A””“文件 报告.docx”。
    pub target_label: String,
    pub resolved_version_id: Option<String>,
    pub retry_of: Option<String>,
    /// 未完成操作是否已由用户处置。
    pub resolved: bool,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub created_at: i64,
    #[cfg_attr(feature = "specta", specta(type = Option<specta_typescript::Number>))]
    pub finished_at: Option<i64>,
    pub message: String,
    pub backup: Option<BackupSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum BackupStatus {
    Creating,
    Ready,
    Failed,
    Cleared,
}

impl BackupStatus {
    fn parse(s: &str) -> Self {
        match s {
            "ready" => BackupStatus::Ready,
            "failed" => BackupStatus::Failed,
            "cleared" => BackupStatus::Cleared,
            _ => BackupStatus::Creating,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BackupSummary {
    pub backup_id: String,
    pub operation_id: String,
    pub reason: String,
    pub status: BackupStatus,
    pub file_count: u32,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub created_at: i64,
    /// 备份文件缺失或校验失败时为 false（显示“不可恢复”）。
    pub recoverable: bool,
    /// 另存覆盖项目外文件时的备份：路径相对于这个文件夹。
    pub external_root: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    Pending,
    Applying,
    Done,
    Failed,
    Skipped,
}

impl ItemState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ItemState::Pending => "pending",
            ItemState::Applying => "applying",
            ItemState::Done => "done",
            ItemState::Failed => "failed",
            ItemState::Skipped => "skipped",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "applying" => ItemState::Applying,
            "done" => ItemState::Done,
            "failed" => ItemState::Failed,
            "skipped" => ItemState::Skipped,
            _ => ItemState::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct OperationItemView {
    pub path: String,
    pub action: Action,
    pub state: ItemState,
    pub backed_up: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct OperationDetail {
    pub summary: OperationSummary,
    pub items: Vec<OperationItemView>,
}

// ───────────────────────── 操作记录的读写 ─────────────────────────

pub(crate) struct NewOp<'a> {
    pub project_id: &'a str,
    pub request_id: &'a str,
    pub op_type: OpType,
    pub target_ref: String,
    pub resolved_version_id: Option<&'a str>,
    pub retry_of: Option<&'a str>,
}

/// 登记新操作。同一请求已存在时返回 `Err(已有操作 ID)`。
pub(crate) fn insert_op(conn: &Connection, op: &NewOp<'_>) -> CoreResult<Result<String, String>> {
    if let Some(existing) = conn
        .query_row(
            "SELECT operation_id FROM operations WHERE project_id = ?1 AND request_id = ?2",
            params![op.project_id, op.request_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(Err(existing));
    }
    let id = new_id();
    conn.execute(
        "INSERT INTO operations (operation_id, project_id, request_id, type, target_ref, resolved_version_id, retry_of, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id,
            op.project_id,
            op.request_id,
            op.op_type.as_str(),
            op.target_ref,
            op.resolved_version_id,
            op.retry_of,
            now_ms()
        ],
    )?;
    Ok(Ok(id))
}

pub(crate) fn set_phase(conn: &Connection, op_id: &str, phase: Phase) -> CoreResult<()> {
    conn.execute("UPDATE operations SET phase = ?2 WHERE operation_id = ?1", params![op_id, phase.as_str()])?;
    Ok(())
}

pub(crate) fn finish_op(conn: &Connection, op_id: &str, status: OpStatus, message: &str) -> CoreResult<()> {
    let message: String = message.chars().take(500).collect();
    conn.execute(
        "UPDATE operations SET status = ?2, result_message = ?3, finished_at = ?4 WHERE operation_id = ?1",
        params![op_id, status.as_str(), message, now_ms()],
    )?;
    Ok(())
}

pub(crate) fn insert_items(conn: &Connection, op_id: &str, items: &[PlanItem]) -> CoreResult<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO operation_items (operation_id, path_key, relative_path, action, entry_type, before_hash, after_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for (i, it) in items.iter().enumerate() {
        // 类型变化时同一路径有“删除旧的”和“新建新的”两项，键上加序号区分
        let key = format!("{}#{i}", it.key);
        stmt.execute(params![
            op_id,
            key,
            it.path,
            it.action.as_str(),
            it.entry_type.as_str(),
            it.before_hash,
            it.after_hash
        ])?;
    }
    Ok(())
}

pub(crate) fn set_item_state(
    conn: &Connection,
    op_id: &str,
    index: usize,
    key: &str,
    state: ItemState,
    error: Option<&CoreError>,
) -> CoreResult<()> {
    conn.execute(
        "UPDATE operation_items SET item_state = ?3, error_code = ?4, error_message = ?5
          WHERE operation_id = ?1 AND path_key = ?2",
        params![
            op_id,
            format!("{key}#{index}"),
            state.as_str(),
            error.map(|e| serde_json::to_value(e.code).ok().and_then(|v| v.as_str().map(str::to_owned))),
            error.map(|e| e.message.clone())
        ],
    )?;
    Ok(())
}

/// 根据逐项状态汇总操作结果。
pub(crate) fn result_of(conn: &Connection, op_id: &str) -> CoreResult<OperationResult> {
    let (status, message): (String, String) =
        conn.query_row("SELECT status, result_message FROM operations WHERE operation_id = ?1", [op_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    let backup_id: Option<String> = conn
        .query_row(
            "SELECT backup_id FROM safety_backups WHERE operation_id = ?1 AND status IN ('ready', 'cleared')",
            [op_id],
            |r| r.get(0),
        )
        .optional()?;
    let mut out = OperationResult {
        operation_id: op_id.to_owned(),
        status: OpStatus::parse(&status),
        message,
        completed: Vec::new(),
        failed: Vec::new(),
        pending: Vec::new(),
        backup_id,
    };
    let mut stmt = conn.prepare(
        "SELECT relative_path, item_state, error_message, action FROM operation_items
          WHERE operation_id = ?1 ORDER BY relative_path, path_key",
    )?;
    let rows = stmt.query_map([op_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, String>(3)?))
    })?;
    for row in rows {
        let (path, state, err, action) = row?;
        if action == "keep" {
            continue;
        }
        match ItemState::parse(&state) {
            ItemState::Done => out.completed.push(path),
            ItemState::Failed => out.failed.push(ItemFailure { path, message: err.unwrap_or_default() }),
            _ => out.pending.push(path),
        }
    }
    Ok(out)
}

/// 标记项目存在未处置的未完成操作。
pub(crate) fn mark_incomplete(conn: &Connection, project_id: &str) -> CoreResult<()> {
    conn.execute("UPDATE projects SET workspace_state = 'incomplete' WHERE project_id = ?1", [project_id])?;
    Ok(())
}

/// 没有其他未处置的未完成操作时，恢复项目的正常状态。
pub(crate) fn refresh_workspace_state(conn: &Connection, project_id: &str) -> CoreResult<()> {
    let open: i64 = conn.query_row(
        "SELECT count(*) FROM operations WHERE project_id = ?1 AND status = 'incomplete' AND resolution = 'open'",
        [project_id],
        |r| r.get(0),
    )?;
    let state = if open > 0 { "incomplete" } else { "normal" };
    conn.execute("UPDATE projects SET workspace_state = ?2 WHERE project_id = ?1", params![project_id, state])?;
    Ok(())
}

const SUMMARY_SQL: &str =
    "SELECT o.operation_id, o.type, o.status, o.target_ref, o.resolved_version_id, o.retry_of, o.resolution,
            o.created_at, o.finished_at, o.result_message,
            b.backup_id, b.reason, b.status, b.expected_file_count, b.created_at, b.external_root
       FROM operations o LEFT JOIN safety_backups b ON b.operation_id = o.operation_id";

fn summary_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<OperationSummary> {
    let backup = match r.get::<_, Option<String>>(10)? {
        Some(backup_id) => Some(BackupSummary {
            backup_id,
            operation_id: r.get(0)?,
            reason: r.get(11)?,
            status: BackupStatus::parse(&r.get::<_, String>(12)?),
            file_count: r.get(13)?,
            created_at: r.get(14)?,
            recoverable: true,
            external_root: r.get(15)?,
        }),
        None => None,
    };
    let target_ref: String = r.get(3)?;
    Ok(OperationSummary {
        operation_id: r.get(0)?,
        op_type: OpType::parse(&r.get::<_, String>(1)?),
        status: OpStatus::parse(&r.get::<_, String>(2)?),
        target_label: restore::TargetRef::parse(&target_ref).label,
        resolved_version_id: r.get(4)?,
        retry_of: r.get(5)?,
        resolved: r.get::<_, String>(6)? == "resolved",
        created_at: r.get(7)?,
        finished_at: r.get(8)?,
        message: r.get(9)?,
        backup,
    })
}

impl Core {
    /// 恢复、切换等写操作的记录，按时间倒序（SRS 3.3.3.2）。没有备份的操作也列出。
    pub fn list_operations(&self, project_id: &str) -> CoreResult<Vec<OperationSummary>> {
        let db = self.db();
        let mut stmt = db.prepare(&format!(
            "{SUMMARY_SQL} WHERE o.project_id = ?1 AND o.type IN ('restore', 'switch')
              ORDER BY o.created_at DESC, o.operation_id DESC"
        ))?;
        let mut list = stmt.query_map([project_id], summary_from_row)?.collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        drop(db);
        for s in &mut list {
            if let Some(b) = &mut s.backup {
                b.recoverable = b.status == BackupStatus::Ready && self.backup_intact(project_id, &b.backup_id)?;
            }
        }
        Ok(list)
    }

    pub fn operation_detail(&self, project_id: &str, operation_id: &str) -> CoreResult<OperationDetail> {
        let mut summary = {
            let db = self.db();
            db.query_row(
                &format!("{SUMMARY_SQL} WHERE o.project_id = ?1 AND o.operation_id = ?2"),
                params![project_id, operation_id],
                summary_from_row,
            )
            .optional()?
            .ok_or_else(|| CoreError::new(ErrorCode::NotFound, "找不到这条操作记录"))?
        };
        if let Some(b) = &mut summary.backup {
            b.recoverable = b.status == BackupStatus::Ready && self.backup_intact(project_id, &b.backup_id)?;
        }
        let db = self.db();
        let mut stmt = db.prepare(
            "SELECT relative_path, action, item_state, backup_file_ref, error_message
               FROM operation_items WHERE operation_id = ?1 ORDER BY relative_path, path_key",
        )?;
        let items = stmt
            .query_map([operation_id], |r| {
                Ok(OperationItemView {
                    path: r.get(0)?,
                    action: Action::parse(&r.get::<_, String>(1)?),
                    state: ItemState::parse(&r.get::<_, String>(2)?),
                    backed_up: r.get::<_, Option<String>>(3)?.is_some(),
                    error: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(OperationDetail { summary, items })
    }

    /// 当前未处置的未完成操作（项目处于 incomplete 时界面据此提示处置）。
    pub fn open_incomplete(&self, project_id: &str) -> CoreResult<Option<OperationSummary>> {
        let db = self.db();
        Ok(db
            .query_row(
                &format!(
                    "{SUMMARY_SQL} WHERE o.project_id = ?1 AND o.status = 'incomplete' AND o.resolution = 'open'
                      ORDER BY o.created_at DESC LIMIT 1"
                ),
                [project_id],
                summary_from_row,
            )
            .optional()?)
    }

    /// 用户确认“保留当前文件状态”，结束对未完成操作的处置（SRS 6.1.1）。
    pub fn resolve_incomplete(&self, project_id: &str, operation_id: &str) -> CoreResult<()> {
        let _guard = self.begin_write(project_id)?;
        let db = self.db();
        let n = db.execute(
            "UPDATE operations SET resolution = 'resolved'
              WHERE project_id = ?1 AND operation_id = ?2 AND status = 'incomplete' AND resolution = 'open'",
            params![project_id, operation_id],
        )?;
        if n == 0 {
            return Err(CoreError::new(ErrorCode::NotFound, "这条操作不需要处置"));
        }
        refresh_workspace_state(&db, project_id)
    }

    /// 安全备份中的文件是否都还在（缺失时界面显示“不可恢复”）。
    fn backup_intact(&self, project_id: &str, backup_id: &str) -> CoreResult<bool> {
        let hashes: Vec<String> = {
            let db = self.db();
            let mut stmt =
                db.prepare("SELECT content_hash FROM backup_files WHERE backup_id = ?1 AND content_hash IS NOT NULL")?;
            stmt.query_map([backup_id], |r| r.get(0))?.collect::<Result<_, _>>()?
        };
        let store = crate::store::ObjectStore::new(&self.project_store_dir(project_id));
        Ok(hashes.iter().all(|h| store.contains(h)))
    }

    /// 上次程序异常退出时进行中的恢复或切换：按实际文件状态核对后记为“未完成”，
    /// 活动方案保持操作前的值，不在后台自动继续写入（SRS 6.1.1，LFVM-AT-03）。
    pub(crate) fn recover_interrupted_ops(&self, project_id: &str) -> CoreResult<()> {
        let (root, ops) = {
            let db = self.db();
            let root = load_project(&db, project_id)?.root;
            let mut stmt = db.prepare(
                "SELECT operation_id, phase FROM operations
                  WHERE project_id = ?1 AND status = 'running' AND type IN ('restore', 'switch')",
            )?;
            let ops: Vec<(String, String)> =
                stmt.query_map([project_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
            (root, ops)
        };
        for (op_id, phase) in ops {
            let db = self.db();
            db.execute(
                "UPDATE safety_backups SET status = 'failed' WHERE operation_id = ?1 AND status = 'creating'",
                [&op_id],
            )?;
            if phase == "planned" || phase == "protecting" {
                // 尚未开始写入工作区
                finish_op(&db, &op_id, OpStatus::Failed, "程序在操作开始写入前意外退出，项目文件夹没有被改动")?;
                continue;
            }
            // 核对正在写入的项：内容已是目标内容记为完成，仍是原内容记为未处理，否则无法确定
            let mut stmt = db.prepare(
                "SELECT path_key, relative_path, before_hash, after_hash, action, entry_type FROM operation_items
                  WHERE operation_id = ?1 AND item_state = 'applying'",
            )?;
            type Row = (String, String, Option<String>, Option<String>, String, String);
            let applying: Vec<Row> = stmt
                .query_map([&op_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
                .collect::<Result<_, _>>()?;
            drop(stmt);
            for (key, rel, before, after, action, entry_type) in applying {
                let state = match RelPath::parse(&rel).map(|r| r.to_path(&root)) {
                    Err(_) => ItemState::Failed,
                    Ok(p) => reconcile(&p, &action, &entry_type, before.as_deref(), after.as_deref()),
                };
                db.execute(
                    "UPDATE operation_items SET item_state = ?3, error_message = CASE WHEN ?3 = 'failed'
                            THEN '程序意外退出，无法确定这个文件是否已写入' ELSE error_message END
                      WHERE operation_id = ?1 AND path_key = ?2",
                    params![op_id, key, state.as_str()],
                )?;
            }
            finish_op(&db, &op_id, OpStatus::Incomplete, "程序在写入过程中意外退出，项目文件夹可能处于新旧混合状态")?;
            mark_incomplete(&db, project_id)?;
        }
        Ok(())
    }
}

/// 核对一个“正在写入”的项在磁盘上的实际状态：已是目标状态为完成，仍是原状态为未处理，否则无法确定。
fn reconcile(
    path: &std::path::Path,
    action: &str,
    entry_type: &str,
    before: Option<&str>,
    after: Option<&str>,
) -> ItemState {
    let meta = std::fs::symlink_metadata(path).ok();
    match (action, entry_type, meta) {
        ("delete", _, None) => ItemState::Done,
        ("create", _, None) => ItemState::Pending,
        ("create", "directory", Some(m)) if m.is_dir() => ItemState::Done,
        ("delete", "directory", Some(m)) if m.is_dir() => ItemState::Pending,
        (_, "file", Some(m)) if m.is_file() => match hash_file(path) {
            Ok((h, _)) if after == Some(h.as_str()) => ItemState::Done,
            Ok((h, _)) if before == Some(h.as_str()) => ItemState::Pending,
            _ => ItemState::Failed,
        },
        _ => ItemState::Failed,
    }
}

#[cfg(test)]
pub(crate) mod failpoint {
    //! 测试用的故障注入点：在指定位置第 n 次经过时返回错误，用于验证失败处理（LFVM-AT-03、AT-04）。
    use std::cell::RefCell;
    use std::collections::HashMap;

    use crate::error::{CoreError, CoreResult, ErrorCode};

    thread_local! {
        static POINTS: RefCell<HashMap<&'static str, (usize, ErrorCode)>> = RefCell::new(HashMap::new());
    }

    pub fn arm(name: &'static str, after: usize, code: ErrorCode) {
        POINTS.with(|p| p.borrow_mut().insert(name, (after, code)));
    }

    pub fn clear() {
        POINTS.with(|p| p.borrow_mut().clear());
    }

    pub fn hit(name: &'static str) -> CoreResult<()> {
        POINTS.with(|p| {
            let mut p = p.borrow_mut();
            if let Some((n, code)) = p.get_mut(name) {
                if *n == 0 {
                    let code = *code;
                    p.remove(name);
                    return Err(CoreError::new(code, format!("（测试注入）{name}")));
                }
                *n -= 1;
            }
            Ok(())
        })
    }
}

#[cfg(not(test))]
pub(crate) mod failpoint {
    #[inline(always)]
    pub fn hit(_: &'static str) -> crate::error::CoreResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
