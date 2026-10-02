//! 方案（SRS 3.3.4.1 创建方案、改名并显示方案走向；3.3.4.2 切换方案前处理未保存变化；规则 R-02）。
//!
//! - 创建方案只登记起点和末端，不修改工作区，也不改变活动方案；
//! - 在方案中保存的版本沿方案延伸，只更新该方案的末端；
//! - 切换方案时若有未保存的变化，只能“保存并继续”或“取消切换”；
//!   切换与整版恢复一样先做安全备份再逐项写入，全部成功后才更新活动方案。

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::content::{SourceRef, check_source};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::ops::OperationResult;
use crate::ops::OpType;
use crate::ops::restore::{ImpactPlan, OnCommit, TargetRef, WorkspaceOp};
use crate::progress::Progress;
use crate::project::{VersionBrief, load_project, version_brief};
use crate::{Core, new_id, now_ms};

pub const MAX_SCHEME_NAME: usize = 100;

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SchemeInfo {
    pub scheme_id: String,
    pub name: String,
    pub base: VersionBrief,
    pub head: VersionBrief,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub created_at: i64,
    /// 是当前活动方案。
    pub active: bool,
    /// 在该方案中保存的版本数。
    pub version_count: u32,
}

/// 方案列表（含默认历史）。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SchemeList {
    pub schemes: Vec<SchemeInfo>,
    /// 默认历史的最新版本；尚无版本时为 null。
    pub default_head: Option<VersionBrief>,
    /// 当前处于默认历史。
    pub default_active: bool,
}

/// 切换目标：某个方案，或默认历史。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SwitchTarget {
    Scheme { scheme_id: String },
    Default,
}

/// 切换前检查（SRS 3.3.4.2 第 1～3 步）。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SwitchCheck {
    /// 有尚未保存的变化：必须先保存或取消切换。
    pub unsaved: bool,
    /// 没有未保存变化时给出影响清单。
    pub impact: Option<ImpactPlan>,
    pub target_label: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SwitchRequest {
    pub project_id: String,
    pub request_id: String,
    pub target: SwitchTarget,
    pub fingerprint: String,
}

/// 方案名称：去除首尾空格后 1～100 字，不含控制字符。
fn normalize_name(name: &str) -> CoreResult<String> {
    let n = name.trim();
    if n.is_empty() {
        return Err(CoreError::new(ErrorCode::InvalidInput, "请填写方案名称"));
    }
    if n.chars().count() > MAX_SCHEME_NAME {
        return Err(CoreError::new(ErrorCode::InvalidInput, format!("方案名称不能超过 {MAX_SCHEME_NAME} 个字")));
    }
    if n.chars().any(char::is_control) {
        return Err(CoreError::new(ErrorCode::InvalidInput, "方案名称不能含有控制字符"));
    }
    Ok(n.to_owned())
}

fn name_key(name: &str) -> String {
    name.to_lowercase()
}

fn ensure_unique(conn: &Connection, project_id: &str, name: &str, except: Option<&str>) -> CoreResult<()> {
    let clash: Option<String> = conn
        .query_row(
            "SELECT scheme_id FROM schemes WHERE project_id = ?1 AND name_key = ?2 AND state = 'active'
               AND scheme_id != coalesce(?3, '')",
            params![project_id, name_key(name), except],
            |r| r.get(0),
        )
        .optional()?;
    if clash.is_some() {
        return Err(CoreError::new(ErrorCode::AlreadyExists, format!("已经有名为“{name}”的方案，请换一个名称")));
    }
    Ok(())
}

pub(crate) fn scheme_head(conn: &Connection, project_id: &str, scheme_id: &str) -> CoreResult<(String, String)> {
    conn.query_row(
        "SELECT name, head_version_id FROM schemes WHERE project_id = ?1 AND scheme_id = ?2 AND state = 'active'",
        params![project_id, scheme_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()?
    .ok_or_else(|| CoreError::new(ErrorCode::NotFound, "找不到这个方案"))
}

impl Core {
    pub fn list_schemes(&self, project_id: &str) -> CoreResult<SchemeList> {
        let db = self.db();
        let project = load_project(&db, project_id)?;
        let mut stmt = db.prepare(
            "SELECT s.scheme_id, s.name, s.base_version_id, s.head_version_id, s.created_at,
                    (SELECT count(*) FROM versions v WHERE v.origin_scheme_id = s.scheme_id)
               FROM schemes s WHERE s.project_id = ?1 AND s.state = 'active' ORDER BY s.created_at, s.scheme_id",
        )?;
        let rows: Vec<(String, String, String, String, i64, u32)> = stmt
            .query_map([project_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
            .collect::<Result<_, _>>()?;
        let schemes = rows
            .into_iter()
            .map(|(id, name, base, head, created_at, n)| {
                Ok(SchemeInfo {
                    active: project.active_scheme_id.as_deref() == Some(id.as_str()),
                    scheme_id: id,
                    name,
                    base: version_brief(&db, &base)?,
                    head: version_brief(&db, &head)?,
                    created_at,
                    version_count: n,
                })
            })
            .collect::<CoreResult<Vec<_>>>()?;
        Ok(SchemeList {
            schemes,
            default_head: project.default_head.as_deref().map(|v| version_brief(&db, v)).transpose()?,
            default_active: project.active_scheme_id.is_none(),
        })
    }

    /// 从任一已保存的版本创建方案。只登记起点和末端，不修改工作区、不改变活动方案（R-02）。
    pub fn create_scheme(&self, project_id: &str, version_id: &str, name: &str) -> CoreResult<SchemeInfo> {
        let name = normalize_name(name)?;
        let _guard = self.begin_write(project_id)?;
        let id = new_id();
        {
            let mut db = self.db();
            let tx = db.transaction()?;
            load_project(&tx, project_id)?;
            check_source(&tx, project_id, &SourceRef::Version { version_id: version_id.to_owned() })
                .map_err(|e| CoreError::new(e.code, format!("不能从这个版本创建方案：{}", e.message)))?;
            ensure_unique(&tx, project_id, &name, None)?;
            tx.execute(
                "INSERT INTO schemes (scheme_id, project_id, name, name_key, base_version_id, head_version_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)",
                params![id, project_id, name, name_key(&name), version_id, now_ms()],
            )?;
            tx.commit()?;
        }
        self.list_schemes(project_id)?
            .schemes
            .into_iter()
            .find(|s| s.scheme_id == id)
            .ok_or_else(|| CoreError::new(ErrorCode::Internal, "方案创建后未找到"))
    }

    /// 改名只改变方案名称，起点、末端和版本都不变。
    pub fn rename_scheme(&self, project_id: &str, scheme_id: &str, name: &str) -> CoreResult<()> {
        let name = normalize_name(name)?;
        let _guard = self.begin_write(project_id)?;
        let db = self.db();
        scheme_head(&db, project_id, scheme_id)?;
        ensure_unique(&db, project_id, &name, Some(scheme_id))?;
        db.execute(
            "UPDATE schemes SET name = ?3, name_key = ?4 WHERE project_id = ?1 AND scheme_id = ?2",
            params![project_id, scheme_id, name, name_key(&name)],
        )?;
        Ok(())
    }

    fn switch_target(&self, project_id: &str, target: &SwitchTarget) -> CoreResult<(String, String)> {
        let db = self.db();
        let project = load_project(&db, project_id)?;
        let (label, version) = match target {
            SwitchTarget::Scheme { scheme_id } => {
                if project.active_scheme_id.as_deref() == Some(scheme_id.as_str()) {
                    return Err(CoreError::new(ErrorCode::InvalidInput, "已经在这个方案中，无需切换"));
                }
                let (name, head) = scheme_head(&db, project_id, scheme_id)?;
                (format!("方案“{name}”"), head)
            }
            SwitchTarget::Default => {
                if project.active_scheme_id.is_none() {
                    return Err(CoreError::new(ErrorCode::InvalidInput, "已经在默认历史中，无需切换"));
                }
                let head = project
                    .default_head
                    .ok_or_else(|| CoreError::new(ErrorCode::NotFound, "默认历史中还没有版本"))?;
                ("默认历史".to_owned(), head)
            }
        };
        check_source(&db, project_id, &SourceRef::Version { version_id: version.clone() })
            .map_err(|e| CoreError::new(e.code, format!("目标的最新版本不可用：{}", e.message)))?;
        Ok((label, version))
    }

    /// 切换前检查：有未保存的变化时只提示保存或取消；否则给出影响清单。
    pub fn check_switch(&self, project_id: &str, target: &SwitchTarget, progress: &dyn Progress) -> CoreResult<SwitchCheck> {
        let (label, version) = self.switch_target(project_id, target)?;
        let changes = self.current_changes(project_id, progress)?;
        if changes.has_changes {
            return Ok(SwitchCheck { unsaved: true, impact: None, target_label: label });
        }
        let impact = self.plan_restore(project_id, &version, progress)?;
        Ok(SwitchCheck { unsaved: false, impact: Some(impact), target_label: label })
    }

    /// 切换方案（SRS 3.3.4.2 第 5、6 步）。全部写入并校验成功后才更新活动方案；
    /// 中途失败时活动方案保持原值，操作记为“未完成”。
    pub fn switch_scheme(&self, req: &SwitchRequest, progress: &dyn Progress) -> CoreResult<OperationResult> {
        let (label, version) = self.switch_target(&req.project_id, &req.target)?;
        // 未保存的变化必须先处理（LFVM-USR-0027）
        if self.current_changes(&req.project_id, progress)?.has_changes {
            return Err(CoreError::new(ErrorCode::UnsavedChanges, "当前有未保存的修改，请先保存为版本或取消切换"));
        }
        let scheme = match &req.target {
            SwitchTarget::Scheme { scheme_id } => Some(scheme_id.clone()),
            SwitchTarget::Default => None,
        };
        let project_id = req.project_id.clone();
        let new_active = scheme.clone();
        let on_commit: OnCommit<'_> = Box::new(move |tx: &Transaction<'_>| {
            tx.execute(
                "UPDATE projects SET active_scheme_id = ?2 WHERE project_id = ?1",
                params![project_id, new_active],
            )?;
            Ok(())
        });
        self.run_workspace_op(
            WorkspaceOp {
                project_id: &req.project_id,
                request_id: &req.request_id,
                op_type: OpType::Switch,
                reason: format!("切换到{label}前"),
                target: TargetRef { label, scheme: Some(scheme.unwrap_or_else(|| "default".into())), path: None },
                version_id: &version,
                fingerprint: &req.fingerprint,
                retry_of: None,
                on_commit: Some(on_commit),
            },
            progress,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::OpStatus;
    use crate::progress::NoProgress;
    use crate::version::SaveRequest;
    use std::path::PathBuf;

    struct Env {
        _tmp: tempfile::TempDir,
        core: Core,
        work: PathBuf,
        pid: String,
    }

    fn env() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let core = Core::open(tmp.path().join("data")).unwrap();
        let work = tmp.path().join("w");
        std::fs::create_dir_all(&work).unwrap();
        let pid = core.add_project(&work, None).unwrap().project_id;
        Env { _tmp: tmp, core, work, pid }
    }

    impl Env {
        fn save(&self) -> String {
            self.core
                .save_version(
                    &SaveRequest { project_id: self.pid.clone(), request_id: new_id(), name: String::new(), note: String::new() },
                    &NoProgress,
                )
                .unwrap()
                .version_id
        }
        fn switch(&self, target: SwitchTarget) -> OperationResult {
            let check = self.core.check_switch(&self.pid, &target, &NoProgress).unwrap();
            assert!(!check.unsaved);
            self.core
                .switch_scheme(
                    &SwitchRequest {
                        project_id: self.pid.clone(),
                        request_id: new_id(),
                        target,
                        fingerprint: check.impact.unwrap().fingerprint,
                    },
                    &NoProgress,
                )
                .unwrap()
        }
        fn read(&self, p: &str) -> String {
            std::fs::read_to_string(self.work.join(p)).unwrap()
        }
    }

    /// AC-0025：从同一版本创建两个不同名方案；有未保存修改时创建方案，工作区和活动方案不变；重名、空名被拒绝。
    #[test]
    fn create_and_rename() {
        let e = env();
        std::fs::write(e.work.join("a.txt"), "1").unwrap();
        let v1 = e.save();
        std::fs::write(e.work.join("a.txt"), "unsaved").unwrap();
        let a = e.core.create_scheme(&e.pid, &v1, " 方案A ").unwrap();
        assert_eq!(a.name, "方案A");
        assert_eq!((a.base.version_id.as_str(), a.head.version_id.as_str()), (v1.as_str(), v1.as_str()));
        assert!(!a.active);
        e.core.create_scheme(&e.pid, &v1, "方案B").unwrap();
        assert_eq!(e.read("a.txt"), "unsaved");
        assert!(e.core.list_schemes(&e.pid).unwrap().default_active);

        assert_eq!(e.core.create_scheme(&e.pid, &v1, "方案a").unwrap_err().code, ErrorCode::AlreadyExists);
        assert_eq!(e.core.create_scheme(&e.pid, &v1, "  ").unwrap_err().code, ErrorCode::InvalidInput);
        assert!(e.core.create_scheme(&e.pid, &v1, &"长".repeat(101)).is_err());

        e.core.rename_scheme(&e.pid, &a.scheme_id, "新名字").unwrap();
        let list = e.core.list_schemes(&e.pid).unwrap();
        let renamed = list.schemes.iter().find(|s| s.scheme_id == a.scheme_id).unwrap();
        assert_eq!(renamed.name, "新名字");
        assert_eq!(renamed.head.version_id, v1);
        assert_eq!(e.core.rename_scheme(&e.pid, &a.scheme_id, "方案B").unwrap_err().code, ErrorCode::AlreadyExists);
        e.core.rename_scheme(&e.pid, &a.scheme_id, "新名字").unwrap();
    }

    /// AC-0027、AC-0006：有未保存变化时不能切换；切换后在方案中保存，只更新方案末端；可切回默认历史。
    #[test]
    fn switch_save_and_back() {
        let e = env();
        std::fs::write(e.work.join("a.txt"), "base").unwrap();
        let v1 = e.save();
        std::fs::write(e.work.join("a.txt"), "default 2").unwrap();
        let v2 = e.save();
        let s = e.core.create_scheme(&e.pid, &v1, "尝试").unwrap();
        let target = SwitchTarget::Scheme { scheme_id: s.scheme_id.clone() };

        // 有未保存的修改
        std::fs::write(e.work.join("a.txt"), "dirty").unwrap();
        assert!(e.core.check_switch(&e.pid, &target, &NoProgress).unwrap().unsaved);
        let err = e.core
            .switch_scheme(&SwitchRequest { project_id: e.pid.clone(), request_id: new_id(), target: target.clone(), fingerprint: String::new() }, &NoProgress)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::UnsavedChanges);
        // 保存并继续：变化保存到当前路线（默认历史）
        let v3 = e.save();

        let r = e.switch(target.clone());
        assert_eq!(r.status, OpStatus::Succeeded, "{r:?}");
        assert_eq!(e.read("a.txt"), "base");
        assert!(r.backup_id.is_some());
        let ov = e.core.overview(&e.pid).unwrap();
        assert_eq!(ov.active_scheme.unwrap().scheme_id, s.scheme_id);
        assert_eq!(ov.baseline.unwrap().version_id, v1);

        // 在方案中保存：父版本为方案末端，只更新方案末端
        std::fs::write(e.work.join("a.txt"), "scheme 1").unwrap();
        let v4 = e.save();
        let list = e.core.list_schemes(&e.pid).unwrap();
        assert_eq!(list.schemes[0].head.version_id, v4);
        assert_eq!(list.default_head.unwrap().version_id, v3);
        let parent: String = e.core.db()
            .query_row("SELECT parent_version_id FROM versions WHERE version_id = ?1", [&v4], |r| r.get(0))
            .unwrap();
        assert_eq!(parent, v1);
        let _ = v2;

        // 切回默认历史
        let r = e.switch(SwitchTarget::Default);
        assert_eq!(r.status, OpStatus::Succeeded);
        assert_eq!(e.read("a.txt"), "dirty");
        assert!(e.core.list_schemes(&e.pid).unwrap().default_active);
        assert_eq!(e.core.check_switch(&e.pid, &SwitchTarget::Default, &NoProgress).unwrap_err().code, ErrorCode::InvalidInput);
    }

    /// 写入中途失败：活动方案保持原值，操作记为“未完成”；重试成功后才更新活动方案。
    #[test]
    fn failed_switch_keeps_active_scheme() {
        let e = env();
        std::fs::write(e.work.join("a.txt"), "1").unwrap();
        std::fs::write(e.work.join("b.txt"), "1").unwrap();
        let v1 = e.save();
        std::fs::write(e.work.join("a.txt"), "2").unwrap();
        std::fs::write(e.work.join("b.txt"), "2").unwrap();
        e.save();
        let s = e.core.create_scheme(&e.pid, &v1, "旧版").unwrap();
        crate::ops::failpoint::arm("execute_item", 1, ErrorCode::Io);
        let r = e.switch(SwitchTarget::Scheme { scheme_id: s.scheme_id.clone() });
        assert_eq!(r.status, OpStatus::Incomplete);
        assert!(e.core.list_schemes(&e.pid).unwrap().default_active, "活动方案保持原值");

        let plan = e.core.plan_retry(&e.pid, &r.operation_id, &NoProgress).unwrap();
        let retry = e.core.retry_operation(&e.pid, &new_id(), &r.operation_id, &plan.fingerprint, &NoProgress).unwrap();
        assert_eq!(retry.status, OpStatus::Succeeded);
        assert!(e.core.list_schemes(&e.pid).unwrap().schemes[0].active);
    }
}
