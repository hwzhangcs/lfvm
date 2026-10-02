//! 项目（SRS 3.3.1.1 加入、打开与移除项目；3.3.1.2 排除项设置）。

use std::path::{Path, PathBuf};

use lfvm_platform::{CasePolicy, EntryKind};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::exclude::{self, ExclusionRule, Matcher, RuleType};
use crate::paths::{self, RelPath, path_key};
use crate::store::ObjectStore;
use crate::{Core, new_id, now_ms};

/// 项目列表中的一项。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ProjectSummary {
    pub project_id: String,
    pub name: String,
    pub root_path: String,
    /// UTC 毫秒。
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub created_at: i64,
    /// 存在未处置的未完成操作（SRS 6.1.1）。
    pub incomplete: bool,
    /// 工作文件夹当前是否仍可访问；不可访问时只能移除。
    pub accessible: bool,
}

/// 加入前的检查结果。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AddCheck {
    pub path: String,
    pub name: String,
    /// 该路径曾作为项目被移除：由用户选择关联原历史或作为新项目（不自动关联）。
    pub previous: Option<PreviousProject>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PreviousProject {
    pub project_id: String,
    pub name: String,
    pub version_count: u32,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct VersionBrief {
    pub version_id: String,
    /// 项目内序号，界面显示为 “V{seq}”。
    pub seq: u32,
    pub name: String,
    pub note: String,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SchemeBrief {
    pub scheme_id: String,
    pub name: String,
}

/// 打开项目后的概览（表 8-3“项目概览”）。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ProjectOverview {
    pub project: ProjectSummary,
    /// 当前活动方案；null 表示处于默认历史。
    pub active_scheme: Option<SchemeBrief>,
    /// 当前比较基准（规则 R-01）；尚无版本时为 null。
    pub baseline: Option<VersionBrief>,
    pub version_count: u32,
}

/// 排除项设置界面中的一条规则。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RuleView {
    pub rule: ExclusionRule,
    /// 当前匹配到的文件数。
    pub match_count: u32,
}

/// 项目目录树中的一个子项（用于在界面中选择要排除的文件或文件夹）。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DirChild {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    /// 按已保存的规则是否被排除。
    pub excluded: bool,
}

/// 数据库中的项目行。
#[derive(Debug, Clone)]
pub(crate) struct ProjectRow {
    pub project_id: String,
    pub name: String,
    pub root: PathBuf,
    pub policy: CasePolicy,
    pub active_scheme_id: Option<String>,
    pub default_head: Option<String>,
    pub incomplete: bool,
    pub created_at: i64,
}

impl ProjectRow {
    pub(crate) fn summary(&self) -> ProjectSummary {
        ProjectSummary {
            project_id: self.project_id.clone(),
            name: self.name.clone(),
            root_path: self.root.to_string_lossy().into_owned(),
            created_at: self.created_at,
            incomplete: self.incomplete,
            accessible: self.root.is_dir(),
        }
    }
}

/// 读取已登记的项目。
pub(crate) fn load_project(conn: &Connection, project_id: &str) -> CoreResult<ProjectRow> {
    conn.query_row(
        "SELECT project_id, name, root_path, case_policy, active_scheme_id, default_head_version_id,
                workspace_state, created_at
           FROM projects WHERE project_id = ?1 AND is_registered = 1",
        [project_id],
        |r| {
            Ok(ProjectRow {
                project_id: r.get(0)?,
                name: r.get(1)?,
                root: PathBuf::from(r.get::<_, String>(2)?),
                policy: CasePolicy::parse(&r.get::<_, String>(3)?).unwrap_or(CasePolicy::Insensitive),
                active_scheme_id: r.get(4)?,
                default_head: r.get(5)?,
                incomplete: r.get::<_, String>(6)? == "incomplete",
                created_at: r.get(7)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| CoreError::new(ErrorCode::NotFound, "项目不存在或已被移除"))
}

/// 当前比较基准（规则 R-01）：活动方案末端；处于默认历史时为默认历史末端。
pub(crate) fn baseline_id(conn: &Connection, p: &ProjectRow) -> CoreResult<Option<String>> {
    match &p.active_scheme_id {
        Some(sid) => Ok(conn
            .query_row("SELECT head_version_id FROM schemes WHERE scheme_id = ?1", [sid], |r| r.get(0))
            .optional()?),
        None => Ok(p.default_head.clone()),
    }
}

pub(crate) fn version_brief(conn: &Connection, version_id: &str) -> CoreResult<VersionBrief> {
    Ok(conn.query_row(
        "SELECT version_id, seq, name, note, created_at FROM versions WHERE version_id = ?1",
        [version_id],
        |r| {
            Ok(VersionBrief {
                version_id: r.get(0)?,
                seq: r.get(1)?,
                name: r.get(2)?,
                note: r.get(3)?,
                created_at: r.get(4)?,
            })
        },
    )?)
}

/// 比较不同项目根目录时使用的键：按当前系统的默认大小写策略，宁可多判为重叠。
fn root_key(path: &Path) -> String {
    path_key(&path.to_string_lossy(), lfvm_platform::default_case_policy())
}

impl Core {
    /// 列出已登记的项目，按加入时间先后排列。
    pub fn list_projects(&self) -> CoreResult<Vec<ProjectSummary>> {
        let ids: Vec<String> = {
            let db = self.db();
            let mut stmt =
                db.prepare("SELECT project_id FROM projects WHERE is_registered = 1 ORDER BY created_at, project_id")?;
            stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?
        };
        let db = self.db();
        ids.iter().map(|id| Ok(load_project(&db, id)?.summary())).collect()
    }

    /// 校验所选目录能否作为项目（SRS 3.3.1.1 第 2 步），并检查是否有曾被移除的同路径项目。
    pub fn check_new_project(&self, path: &Path) -> CoreResult<AddCheck> {
        let root = self.validate_new_root(path)?;
        let key = root_key(&root);
        let previous = {
            let db = self.db();
            db.query_row(
                "SELECT p.project_id, p.name, p.created_at,
                        (SELECT count(*) FROM versions v WHERE v.project_id = p.project_id)
                   FROM projects p WHERE p.root_key = ?1 AND p.is_registered = 0
                  ORDER BY p.created_at DESC LIMIT 1",
                [&key],
                |r| {
                    Ok(PreviousProject {
                        project_id: r.get(0)?,
                        name: r.get(1)?,
                        created_at: r.get(2)?,
                        version_count: r.get(3)?,
                    })
                },
            )
            .optional()?
        };
        Ok(AddCheck { path: root.to_string_lossy().into_owned(), name: folder_name(&root), previous })
    }

    /// 加入项目。`associate_previous` 为用户选择关联的原历史（来自 [`AddCheck::previous`]）。
    /// 加入时不修改用户文件，也不自动保存版本。
    pub fn add_project(&self, path: &Path, associate_previous: Option<&str>) -> CoreResult<ProjectSummary> {
        let root = self.validate_new_root(path)?;
        let policy = lfvm_platform::probe_case_policy(&root).map_err(|e| CoreError::io(e, &root))?;
        let key = root_key(&root);
        let root_str = root.to_string_lossy().into_owned();

        let mut db = self.db();
        let tx = db.transaction()?;
        let project_id = match associate_previous {
            Some(prev) => {
                let n = tx.execute(
                    "UPDATE projects SET is_registered = 1, root_path = ?2, case_policy = ?3
                      WHERE project_id = ?1 AND is_registered = 0 AND root_key = ?4",
                    params![prev, root_str, policy.as_str(), key],
                )?;
                if n != 1 {
                    return Err(CoreError::new(ErrorCode::NotFound, "找不到可以关联的原历史"));
                }
                prev.to_owned()
            }
            None => {
                let id = new_id();
                tx.execute(
                    "INSERT INTO projects (project_id, name, root_path, root_key, case_policy, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![id, folder_name(&root), root_str, key, policy.as_str(), now_ms()],
                )?;
                exclude::replace_rules(&tx, &id, &exclude::system_default_rules(), policy)?;
                id
            }
        };
        tx.commit()?;
        let row = load_project(&db, &project_id)?;
        drop(db);
        ObjectStore::new(&self.project_store_dir(&project_id)).ensure_dirs()?;
        Ok(row.summary())
    }

    /// 从列表中移除项目：只标记为未登记，工作文件夹和本地历史都保留（LFVM-Q-01）。
    /// 目录已失效的项目也可以移除。
    pub fn remove_project(&self, project_id: &str) -> CoreResult<()> {
        let _guard = self.begin_write(project_id)?;
        let n = self.db().execute(
            "UPDATE projects SET is_registered = 0 WHERE project_id = ?1 AND is_registered = 1",
            [project_id],
        )?;
        if n == 0 {
            return Err(CoreError::new(ErrorCode::NotFound, "项目不存在或已被移除"));
        }
        Ok(())
    }

    /// 打开项目：检查目录可访问，处理上次异常退出遗留的保存操作，返回概览。
    pub fn open_project(&self, project_id: &str) -> CoreResult<ProjectOverview> {
        let row = load_project(&self.db(), project_id)?;
        if !row.root.is_dir() {
            return Err(CoreError::new(
                ErrorCode::NotFound,
                "项目文件夹无法访问，可能已被移动、改名或删除。可以把它从列表中移除，再把新位置作为新项目加入",
            )
            .with_path(&row.root));
        }
        self.recover_interrupted_saves(project_id)?;
        self.recover_interrupted_ops(project_id)?;
        self.overview(project_id)
    }

    pub fn overview(&self, project_id: &str) -> CoreResult<ProjectOverview> {
        let db = self.db();
        let row = load_project(&db, project_id)?;
        let active_scheme = match &row.active_scheme_id {
            Some(sid) => db
                .query_row("SELECT scheme_id, name FROM schemes WHERE scheme_id = ?1", [sid], |r| {
                    Ok(SchemeBrief { scheme_id: r.get(0)?, name: r.get(1)? })
                })
                .optional()?,
            None => None,
        };
        let baseline = baseline_id(&db, &row)?.map(|v| version_brief(&db, &v)).transpose()?;
        let version_count =
            db.query_row("SELECT count(*) FROM versions WHERE project_id = ?1", [project_id], |r| r.get(0))?;
        Ok(ProjectOverview { project: row.summary(), active_scheme, baseline, version_count })
    }

    /// 已保存的排除规则及每条当前匹配的文件数。
    pub fn exclusion_rules(&self, project_id: &str) -> CoreResult<Vec<RuleView>> {
        let rules = exclude::load_rules(&self.db(), project_id)?;
        let counts = self.count_rule_matches(project_id, &rules)?;
        Ok(rules.into_iter().zip(counts).map(|(rule, match_count)| RuleView { rule, match_count }).collect())
    }

    /// 统计每条规则在当前工作区匹配到的文件数（编辑中的规则也可以统计）。
    pub fn count_rule_matches(&self, project_id: &str, rules: &[ExclusionRule]) -> CoreResult<Vec<u32>> {
        let row = load_project(&self.db(), project_id)?;
        let mut counts = vec![0u32; rules.len()];
        walk_files(&row.root, &mut |rel| {
            for (i, r) in rules.iter().enumerate() {
                if Matcher::rule_matches(r, rel, false, row.policy) {
                    counts[i] += 1;
                }
            }
        })?;
        Ok(counts)
    }

    /// 保存完整的规则集合。只影响之后的扫描和操作，既有版本不变。
    pub fn save_exclusion_rules(&self, project_id: &str, rules: &[ExclusionRule]) -> CoreResult<()> {
        let _guard = self.begin_write(project_id)?;
        let row = load_project(&self.db(), project_id)?;
        let mut normalized = Vec::with_capacity(rules.len());
        let mut seen = std::collections::HashSet::new();
        for r in rules {
            let n = exclude::normalize_rule(r)?;
            if n.entry_type != RuleType::NamePattern {
                let rel = RelPath::parse(&n.relative_path)?;
                let abs = rel.to_path(&row.root);
                if !abs.starts_with(&row.root) {
                    return Err(CoreError::new(ErrorCode::PathOutOfScope, "排除项必须位于项目文件夹内"));
                }
            }
            if seen.insert((n.entry_type, path_key(&n.relative_path, row.policy))) {
                normalized.push(n);
            }
        }
        for d in exclude::SYSTEM_DEFAULT_PATTERNS {
            if !normalized.iter().any(|r| r.is_system_default && r.relative_path == d) {
                return Err(CoreError::new(ErrorCode::InvalidInput, "系统默认规则不能删除，只能停用"));
            }
        }
        let mut db = self.db();
        let tx = db.transaction()?;
        exclude::replace_rules(&tx, project_id, &normalized, row.policy)?;
        tx.commit()?;
        Ok(())
    }

    /// 列出项目中某个文件夹的直接子项（`dir` 为 None 时列根目录）。不跟随链接。
    pub fn list_workspace_dir(&self, project_id: &str, dir: Option<&str>) -> CoreResult<Vec<DirChild>> {
        let (row, rules) = {
            let db = self.db();
            let row = load_project(&db, project_id)?;
            let rules = exclude::load_rules(&db, project_id)?;
            (row, rules)
        };
        let matcher = Matcher::new(&rules, row.policy)?;
        let parent = dir.map(RelPath::parse).transpose()?;
        let abs = parent.as_ref().map_or_else(|| row.root.clone(), |p| p.to_path(&row.root));
        let mut out = Vec::new();
        for item in std::fs::read_dir(&abs).map_err(|e| CoreError::io(e, &abs))? {
            let item = item.map_err(|e| CoreError::io(e, &abs))?;
            let Some(name) = item.file_name().to_str().map(str::to_owned) else { continue };
            let Ok(rel) = RelPath::child(parent.as_ref(), &name) else { continue };
            if rel.file_name().starts_with(exclude::INTERNAL_TMP_PREFIX) {
                continue;
            }
            let meta = std::fs::symlink_metadata(item.path()).map_err(|e| CoreError::io(e, item.path()))?;
            let is_dir = lfvm_platform::classify(&meta) == EntryKind::Dir;
            out.push(DirChild { excluded: matcher.is_excluded(&rel, is_dir), path: rel.to_string(), name, is_dir });
        }
        out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
        Ok(out)
    }

    /// 校验目录：存在且可读；不是磁盘根目录或系统目录；不是链接；
    /// 不与已登记项目或历史存储目录相同或相互包含。返回规范化后的真实路径。
    fn validate_new_root(&self, path: &Path) -> CoreResult<PathBuf> {
        let meta = std::fs::symlink_metadata(path).map_err(|e| CoreError::io(e, path))?;
        match lfvm_platform::classify(&meta) {
            EntryKind::Dir => {}
            EntryKind::Link => {
                return Err(CoreError::new(
                    ErrorCode::LinkNotFollowed,
                    "所选位置是一个快捷链接（符号链接或目录联接），请选择它指向的文件夹本身",
                )
                .with_path(path));
            }
            EntryKind::CloudPlaceholder => {
                return Err(CoreError::new(ErrorCode::CloudPlaceholder, "所选文件夹只存在于云端，请先下载到本机")
                    .with_path(path));
            }
            _ => return Err(CoreError::new(ErrorCode::InvalidInput, "请选择一个文件夹").with_path(path)),
        }
        let root = paths::canonical(path)?;
        std::fs::read_dir(&root).map_err(|e| CoreError::io(e, &root))?;

        let policy = lfvm_platform::default_case_policy();
        if lfvm_platform::is_filesystem_root(&root) {
            return Err(CoreError::new(
                ErrorCode::ProtectedDirectory,
                "不能把整个磁盘作为项目，请选择其中的一个文件夹",
            )
            .with_path(&root));
        }
        for sys in lfvm_platform::protected_dirs() {
            let sys = paths::canonical(&sys).unwrap_or(sys);
            if paths::is_inside(&root, &sys, policy) {
                return Err(CoreError::new(ErrorCode::ProtectedDirectory, "不能把系统文件夹作为项目").with_path(&root));
            }
        }
        if paths::same_or_nested(&root, self.data_dir(), policy) {
            return Err(CoreError::new(ErrorCode::DirectoryOverlap, "所选文件夹与本软件的历史存储位置相同或相互包含")
                .with_path(&root));
        }
        for p in self.list_projects()? {
            if paths::same_or_nested(&root, Path::new(&p.root_path), policy) {
                let msg = if root_key(&root) == root_key(Path::new(&p.root_path)) {
                    format!("这个文件夹已经是项目“{}”", p.name)
                } else {
                    format!("所选文件夹与已有项目“{}”相互包含，不能重复管理", p.name)
                };
                return Err(CoreError::new(ErrorCode::DirectoryOverlap, msg).with_path(&root));
            }
        }
        Ok(root)
    }
}

fn folder_name(root: &Path) -> String {
    let name = root.file_name().map_or_else(|| root.to_string_lossy(), |n| n.to_string_lossy()).into_owned();
    name.chars().take(100).collect()
}

/// 遍历工作区中的全部普通文件（不跟随链接，不计算摘要）。
fn walk_files(root: &Path, f: &mut dyn FnMut(&RelPath)) -> CoreResult<()> {
    let mut stack: Vec<Option<RelPath>> = vec![None];
    while let Some(dir) = stack.pop() {
        let abs = dir.as_ref().map_or_else(|| root.to_path_buf(), |d| d.to_path(root));
        let Ok(rd) = std::fs::read_dir(&abs) else { continue };
        for item in rd.flatten() {
            let Some(name) = item.file_name().to_str().map(str::to_owned) else { continue };
            let Ok(rel) = RelPath::child(dir.as_ref(), &name) else { continue };
            let Ok(meta) = std::fs::symlink_metadata(item.path()) else { continue };
            match lfvm_platform::classify(&meta) {
                EntryKind::Dir => stack.push(Some(rel)),
                EntryKind::File => f(&rel),
                _ => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Env {
        _tmp: tempfile::TempDir,
        core: Core,
        work: PathBuf,
    }

    fn env() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let core = Core::open(tmp.path().join("data")).unwrap();
        let work = tmp.path().join("作业");
        std::fs::create_dir_all(work.join("sub")).unwrap();
        std::fs::write(work.join("a.txt"), b"a").unwrap();
        Env { _tmp: tmp, core, work }
    }

    #[test]
    fn add_list_remove_keeps_files() {
        let e = env();
        let check = e.core.check_new_project(&e.work).unwrap();
        assert_eq!(check.name, "作业");
        assert!(check.previous.is_none());
        let p = e.core.add_project(&e.work, None).unwrap();
        assert_eq!(e.core.list_projects().unwrap().len(), 1);
        // 系统默认排除规则已建立
        assert_eq!(e.core.exclusion_rules(&p.project_id).unwrap().len(), 3);

        e.core.remove_project(&p.project_id).unwrap();
        assert!(e.core.list_projects().unwrap().is_empty());
        assert!(e.work.join("a.txt").is_file());
    }

    #[test]
    fn readd_offers_previous_history_without_auto_linking() {
        let e = env();
        let p = e.core.add_project(&e.work, None).unwrap();
        e.core.remove_project(&p.project_id).unwrap();
        let check = e.core.check_new_project(&e.work).unwrap();
        assert_eq!(check.previous.as_ref().unwrap().project_id, p.project_id);

        let again = e.core.add_project(&e.work, Some(&p.project_id)).unwrap();
        assert_eq!(again.project_id, p.project_id);
        e.core.remove_project(&again.project_id).unwrap();
        let fresh = e.core.add_project(&e.work, None).unwrap();
        assert_ne!(fresh.project_id, p.project_id);
    }

    #[test]
    fn rejects_duplicates_nesting_and_data_dir() {
        let e = env();
        e.core.add_project(&e.work, None).unwrap();
        for bad in [e.work.clone(), e.work.join("sub"), e.work.parent().unwrap().to_path_buf()] {
            let err = e.core.check_new_project(&bad).unwrap_err();
            assert_eq!(err.code, ErrorCode::DirectoryOverlap, "{bad:?}");
        }
        let err = e.core.check_new_project(e.core.data_dir()).unwrap_err();
        assert_eq!(err.code, ErrorCode::DirectoryOverlap);
        assert_eq!(e.core.check_new_project(Path::new("/")).unwrap_err().code, ErrorCode::ProtectedDirectory);
        assert_eq!(e.core.check_new_project(&e.work.join("nope")).unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(e.core.check_new_project(&e.work.join("a.txt")).unwrap_err().code, ErrorCode::InvalidInput);
    }

    #[test]
    fn exclusion_rules_roundtrip_and_validation() {
        let e = env();
        let p = e.core.add_project(&e.work, None).unwrap();
        let mut rules: Vec<ExclusionRule> =
            e.core.exclusion_rules(&p.project_id).unwrap().into_iter().map(|v| v.rule).collect();
        rules.push(ExclusionRule {
            relative_path: "sub".into(),
            entry_type: RuleType::Directory,
            is_system_default: false,
            enabled: true,
        });
        e.core.save_exclusion_rules(&p.project_id, &rules).unwrap();
        assert_eq!(e.core.exclusion_rules(&p.project_id).unwrap().len(), 4);

        // 删除系统默认规则被拒绝
        let without_default: Vec<_> = rules.iter().skip(1).cloned().collect();
        assert!(e.core.save_exclusion_rules(&p.project_id, &without_default).is_err());

        let kids = e.core.list_workspace_dir(&p.project_id, None).unwrap();
        assert_eq!(kids[0].name, "sub");
        assert!(kids[0].excluded);
        assert!(!kids[1].excluded);
    }

    #[test]
    fn open_reports_missing_folder() {
        let e = env();
        let p = e.core.add_project(&e.work, None).unwrap();
        std::fs::rename(&e.work, e.work.with_file_name("moved")).unwrap();
        assert_eq!(e.core.open_project(&p.project_id).unwrap_err().code, ErrorCode::NotFound);
        assert!(!e.core.list_projects().unwrap()[0].accessible);
        e.core.remove_project(&p.project_id).unwrap();
    }
}
