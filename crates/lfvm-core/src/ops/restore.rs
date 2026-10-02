//! 恢复整个版本（SRS 3.3.2.5），以及切换方案（3.3.4.2）共用的工作区写入流程。
//!
//! 流程：影响清单 → 用户确认 → 重新计算并核对指纹 → 排除冲突检查 → 空间检查 →
//! 安全备份 → 逐项写入 → 提交（切换方案时更新活动方案）。

use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::changes::load_manifest;
use crate::content::{SourceRef, check_source};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::exclude::{self, Matcher};
use crate::ops::apply::ExecOutcome;
use crate::ops::backup::{BackupRequest, ensure_space};
use crate::ops::plan::{self, Action, Plan, PlanItem};
use crate::ops::{
    NewOp, OpStatus, OpType, OperationResult, Phase, finish_op, insert_items, insert_op, mark_incomplete,
    refresh_workspace_state, result_of, set_phase,
};
use crate::progress::Progress;
use crate::project::{ProjectRow, VersionBrief, load_project, version_brief};
use crate::scan::{self, ScanProblem};
use crate::store::ObjectStore;
use crate::Core;

#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PlanCounts {
    pub create: u32,
    pub replace: u32,
    pub delete: u32,
    pub keep: u32,
    pub type_change: u32,
    /// 需要备份的现存文件数（0 表示“无需备份”）。
    pub backup_files: u32,
}

/// 影响清单（供用户确认）。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ImpactPlan {
    pub target: VersionBrief,
    pub items: Vec<PlanItem>,
    pub counts: PlanCounts,
    /// 排除冲突、链接等问题；存在时不能执行。
    pub conflicts: Vec<ScanProblem>,
    /// 用户确认时交回，用于确认文件夹在确认后没有再变化。
    pub fingerprint: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct WorkspaceOpRequest {
    pub project_id: String,
    pub request_id: String,
    pub version_id: String,
    pub fingerprint: String,
}

/// 记录在操作中的对象说明（JSON），既用于显示，也用于“重试”时找回原目标。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TargetRef {
    pub label: String,
    /// 切换方案时的目标方案；"default" 表示默认历史。
    #[serde(default)]
    pub scheme: Option<String>,
    /// 展开、导出时的输出文件夹。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl TargetRef {
    pub(crate) fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub(crate) fn parse(s: &str) -> TargetRef {
        serde_json::from_str(s).unwrap_or(TargetRef { label: s.to_owned(), scheme: None, path: None })
    }
}

pub(crate) type OnCommit<'a> = Box<dyn FnOnce(&Transaction<'_>) -> CoreResult<()> + 'a>;

pub(crate) struct WorkspaceOp<'a> {
    pub project_id: &'a str,
    pub request_id: &'a str,
    pub op_type: OpType,
    pub target: TargetRef,
    pub version_id: &'a str,
    pub reason: String,
    pub fingerprint: &'a str,
    pub retry_of: Option<&'a str>,
    pub on_commit: Option<OnCommit<'a>>,
}

pub(crate) struct Prepared {
    pub project: ProjectRow,
    pub plan: Plan,
}

fn counts(plan: &Plan) -> PlanCounts {
    let n = |a: Action| plan.count(a) as u32;
    PlanCounts {
        create: n(Action::Create),
        replace: n(Action::Replace),
        delete: n(Action::Delete),
        keep: n(Action::Keep),
        type_change: plan.items.iter().filter(|i| i.type_change && i.action == Action::Create).count() as u32,
        backup_files: plan
            .items
            .iter()
            .filter(|i| matches!(i.action, Action::Replace | Action::Delete) && !i.entry_type.is_dir())
            .count() as u32,
    }
}

impl Core {
    /// 扫描工作区并计算把它变成目标版本的影响清单。
    pub(crate) fn prepare_workspace_plan(
        &self,
        project_id: &str,
        version_id: &str,
        progress: &dyn Progress,
    ) -> CoreResult<Prepared> {
        let (project, rules, cache, target) = {
            let db = self.db();
            let project = load_project(&db, project_id)?;
            check_source(&db, project_id, &SourceRef::Version { version_id: version_id.to_owned() })?;
            (project, exclude::load_rules(&db, project_id)?, scan::load_cache(&db, project_id)?, load_manifest(&db, version_id)?)
        };
        if !project.root.is_dir() {
            return Err(CoreError::new(ErrorCode::NotFound, "项目文件夹无法访问").with_path(&project.root));
        }
        let matcher = Matcher::new(&rules, project.policy)?;
        let current = scan::scan(&project.root, &matcher, project.policy, &cache, progress)?;
        scan::store_cache(&mut self.db(), project_id, &current, project.policy)?;
        let mut plan = plan::compute(&current, &matcher, &target, &project.root);
        plan.conflicts.extend(current.problems);
        Ok(Prepared { project, plan })
    }

    /// 恢复整个版本前的影响清单（SRS 3.3.2.5 第 2 步）。只读，不改动任何文件。
    pub fn plan_restore(&self, project_id: &str, version_id: &str, progress: &dyn Progress) -> CoreResult<ImpactPlan> {
        let p = self.prepare_workspace_plan(project_id, version_id, progress)?;
        Ok(ImpactPlan {
            target: version_brief(&self.db(), version_id)?,
            counts: counts(&p.plan),
            fingerprint: p.plan.fingerprint(),
            conflicts: p.plan.conflicts,
            items: p.plan.items,
        })
    }

    /// 恢复整个版本。整版恢复不要求先保存当前变化，不改变任何路线的末端（规则 R-01）。
    pub fn restore_version(&self, req: &WorkspaceOpRequest, progress: &dyn Progress) -> CoreResult<OperationResult> {
        let label = format!("版本 V{}", version_brief(&self.db(), &req.version_id)?.seq);
        self.run_workspace_op(
            WorkspaceOp {
                project_id: &req.project_id,
                request_id: &req.request_id,
                op_type: OpType::Restore,
                reason: format!("恢复到{label}前"),
                target: TargetRef { label, scheme: None, path: None },
                version_id: &req.version_id,
                fingerprint: &req.fingerprint,
                retry_of: None,
                on_commit: None,
            },
            progress,
        )
    }

    /// 未完成操作的影响清单：以原操作开始时确定的目标版本为准（表 4-8 resolved_version_id）。
    pub fn plan_retry(&self, project_id: &str, operation_id: &str, progress: &dyn Progress) -> CoreResult<ImpactPlan> {
        let (version, _, _) = self.retry_target(project_id, operation_id)?;
        self.plan_restore(project_id, &version, progress)
    }

    /// 重新扫描后重试未完成的操作：生成新的操作记录并关联原记录，原记录状态不变（LFVM-AT-10）。
    pub fn retry_operation(
        &self,
        project_id: &str,
        request_id: &str,
        operation_id: &str,
        fingerprint: &str,
        progress: &dyn Progress,
    ) -> CoreResult<OperationResult> {
        let (version, op_type, target) = self.retry_target(project_id, operation_id)?;
        let scheme = target.scheme.clone();
        let on_commit: Option<OnCommit<'_>> = match (op_type, scheme) {
            (OpType::Switch, Some(s)) => Some(Box::new(move |tx: &Transaction<'_>| {
                let active = if s == "default" { None } else { Some(s) };
                tx.execute("UPDATE projects SET active_scheme_id = ?2 WHERE project_id = ?1", params![project_id, active])?;
                Ok(())
            })),
            _ => None,
        };
        self.run_workspace_op(
            WorkspaceOp {
                project_id,
                request_id,
                op_type,
                reason: format!("重试：{}", if op_type == OpType::Switch { format!("切换到{}前", target.label) } else { format!("恢复到{}前", target.label) }),
                target,
                version_id: &version,
                fingerprint,
                retry_of: Some(operation_id),
                on_commit,
            },
            progress,
        )
    }

    fn retry_target(&self, project_id: &str, operation_id: &str) -> CoreResult<(String, OpType, TargetRef)> {
        let db = self.db();
        let row: Option<(Option<String>, String, String)> = db
            .query_row(
                "SELECT resolved_version_id, type, target_ref FROM operations
                  WHERE project_id = ?1 AND operation_id = ?2 AND status = 'incomplete' AND type IN ('restore', 'switch')",
                params![project_id, operation_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        match row {
            Some((Some(v), t, target)) => {
                Ok((v, if t == "switch" { OpType::Switch } else { OpType::Restore }, TargetRef::parse(&target)))
            }
            _ => Err(CoreError::new(ErrorCode::NotFound, "这条操作不能重试（只有未完成的整版恢复或切换可以重试）")),
        }
    }

    /// 整版恢复与切换方案共用的执行流程。写入前的问题记为“失败”（工作区未改动），写入中途的问题记为“未完成”。
    pub(crate) fn run_workspace_op(&self, op: WorkspaceOp<'_>, progress: &dyn Progress) -> CoreResult<OperationResult> {
        let _guard = self.begin_write(op.project_id)?;
        let op_id = {
            let db = self.db();
            let project = load_project(&db, op.project_id)?;
            if project.incomplete && op.retry_of.is_none() {
                return Err(CoreError::new(
                    ErrorCode::IncompleteOperation,
                    "有未完成的操作需要先处理：可以从安全备份找回文件、重新扫描后重试，或确认保留当前文件状态",
                ));
            }
            match insert_op(
                &db,
                &NewOp {
                    project_id: op.project_id,
                    request_id: op.request_id,
                    op_type: op.op_type,
                    target_ref: op.target.to_json(),
                    resolved_version_id: Some(op.version_id),
                    retry_of: op.retry_of,
                },
            )? {
                Ok(id) => id,
                // 同一请求重复提交：返回第一次的结果，不重复执行（LFVM-AT-13）
                Err(existing) => return result_of(&db, &existing),
            }
        };
        let store = ObjectStore::new(&self.project_store_dir(op.project_id));
        let fail = |status: OpStatus, msg: &str| -> CoreResult<OperationResult> {
            let db = self.db();
            finish_op(&db, &op_id, status, msg)?;
            result_of(&db, &op_id)
        };

        // 1. 重新计算影响清单，确认与用户看到的一致
        let prep = match self.prepare_workspace_plan(op.project_id, op.version_id, progress) {
            Ok(p) => p,
            Err(e) if e.code == ErrorCode::Cancelled => return fail(OpStatus::Cancelled, "已取消"),
            Err(e) => return fail(OpStatus::Failed, &e.message),
        };
        insert_items(&self.db(), &op_id, &prep.plan.items)?;
        if prep.plan.fingerprint() != op.fingerprint {
            return fail(OpStatus::Failed, "项目文件夹在确认后又发生了变化，请重新查看影响清单后再确认");
        }
        if let Some(c) = prep.plan.conflicts.first() {
            return fail(OpStatus::Failed, &format!("{}：{}（项目文件夹没有被改动）", c.path, c.message));
        }
        let write_bytes: u64 = prep
            .plan
            .items
            .iter()
            .filter(|i| matches!(i.action, Action::Create | Action::Replace))
            .map(|i| i.after_size.unwrap_or(0) as u64)
            .sum();
        if let Err(e) = ensure_space(&prep.project.root, write_bytes, "项目文件夹") {
            return fail(OpStatus::Failed, &e.message);
        }

        // 2. 安全备份
        set_phase(&self.db(), &op_id, Phase::Protecting)?;
        let backup = self.create_backup(
            &BackupRequest {
                project_id: op.project_id,
                operation_id: &op_id,
                reason: op.reason.clone(),
                root: &prep.project.root,
                external_root: None,
                items: &prep.plan.items,
            },
            progress,
        );
        let backup_note = match backup {
            Ok(Some(_)) => "",
            Ok(None) => "（只新增文件，无需备份）",
            Err(e) => return fail(OpStatus::Failed, &e.message),
        };
        if progress.is_cancelled() {
            return fail(OpStatus::Cancelled, "已取消，项目文件夹没有被改动");
        }

        // 3. 逐项写入
        set_phase(&self.db(), &op_id, Phase::Executing)?;
        let ordered = prep.plan.ordered();
        match self.execute_items(&op_id, &prep.project.root, &store, &ordered, progress) {
            ExecOutcome::Completed => {}
            ExecOutcome::Stopped { error, touched } => {
                let db = self.db();
                if !touched {
                    let status = if error.is_none() { OpStatus::Cancelled } else { OpStatus::Failed };
                    let msg = error.map_or("已取消，项目文件夹没有被改动".to_owned(), |e| format!("{}（项目文件夹没有被改动）", e.message));
                    finish_op(&db, &op_id, status, &msg)?;
                } else {
                    let msg = match error {
                        Some(e) => format!("写入中途出错，已停止：{}。项目文件夹处于新旧混合状态，可以从安全备份找回文件，或处理后重试", e.message),
                        None => "已按要求停止，项目文件夹处于新旧混合状态，可以从安全备份找回文件，或稍后重试".to_owned(),
                    };
                    finish_op(&db, &op_id, OpStatus::Incomplete, &msg)?;
                    mark_incomplete(&db, op.project_id)?;
                }
                return result_of(&db, &op_id);
            }
        }

        // 4. 提交
        set_phase(&self.db(), &op_id, Phase::Committing)?;
        let mut db = self.db();
        let committed = (|| {
            let tx = db.transaction()?;
            if let Some(f) = op.on_commit {
                f(&tx)?;
            }
            finish_op(&tx, &op_id, OpStatus::Succeeded, &format!("已完成{backup_note}"))?;
            if let Some(prev) = op.retry_of {
                tx.execute("UPDATE operations SET resolution = 'resolved' WHERE operation_id = ?1", [prev])?;
            }
            refresh_workspace_state(&tx, op.project_id)?;
            tx.commit()?;
            Ok::<_, CoreError>(())
        })();
        if let Err(e) = committed {
            finish_op(&db, &op_id, OpStatus::Incomplete, &format!("文件已全部写入，但登记失败：{}", e.message))?;
            mark_incomplete(&db, op.project_id)?;
        }
        result_of(&db, &op_id)
    }
}
