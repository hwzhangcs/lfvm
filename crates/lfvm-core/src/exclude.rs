//! 排除规则（SRS 3.3.1.2 设置排除的文件或文件夹）。
//!
//! 匹配规则：
//! - 规则以项目内相对路径表示；文件夹规则覆盖该文件夹及其全部子孙，文件规则只匹配该文件；
//! - 按目录边界匹配，“a/b” 不匹配 “a/b2”；大小写按项目所在文件系统的策略；
//! - 系统默认规则是文件名模式（`~$*`、`Thumbs.db`、`desktop.ini`），在任意目录下匹配，只能停用不能删除。

use std::collections::HashSet;

use lfvm_platform::CasePolicy;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::glob::wildcard_match;
use crate::paths::{RelPath, path_key};

/// 系统写入工作区的临时文件前缀。这类文件始终不纳入版本，也不在界面中显示。
pub const INTERNAL_TMP_PREFIX: &str = ".lfvm~";

pub const SYSTEM_DEFAULT_PATTERNS: [&str; 3] = ["~$*", "Thumbs.db", "desktop.ini"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum RuleType {
    File,
    Directory,
    /// 文件名模式，仅用于系统默认规则。
    NamePattern,
}

impl RuleType {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleType::File => "file",
            RuleType::Directory => "directory",
            RuleType::NamePattern => "name_pattern",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "directory" => RuleType::Directory,
            "name_pattern" => RuleType::NamePattern,
            _ => RuleType::File,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ExclusionRule {
    pub relative_path: String,
    pub entry_type: RuleType,
    pub is_system_default: bool,
    pub enabled: bool,
}

pub fn system_default_rules() -> Vec<ExclusionRule> {
    SYSTEM_DEFAULT_PATTERNS
        .iter()
        .map(|p| ExclusionRule {
            relative_path: (*p).to_owned(),
            entry_type: RuleType::NamePattern,
            is_system_default: true,
            enabled: true,
        })
        .collect()
}

/// 校验并规范化一条规则。用户规则必须是项目内的相对路径。
pub fn normalize_rule(rule: &ExclusionRule) -> CoreResult<ExclusionRule> {
    let mut r = rule.clone();
    if r.is_system_default {
        if r.entry_type != RuleType::NamePattern || !SYSTEM_DEFAULT_PATTERNS.contains(&r.relative_path.as_str()) {
            return Err(CoreError::new(ErrorCode::InvalidInput, "系统默认规则不能修改，只能停用"));
        }
        return Ok(r);
    }
    if r.entry_type == RuleType::NamePattern {
        return Err(CoreError::new(ErrorCode::InvalidInput, "只能选择项目中的文件或文件夹作为排除项"));
    }
    r.relative_path = RelPath::parse(&r.relative_path)?.as_str().to_owned();
    Ok(r)
}

/// 判断路径是否被排除。只使用已启用的规则。
#[derive(Debug, Clone)]
pub struct Matcher {
    policy: CasePolicy,
    files: HashSet<String>,
    dirs: Vec<RelPath>,
    patterns: Vec<String>,
}

impl Matcher {
    pub fn new(rules: &[ExclusionRule], policy: CasePolicy) -> CoreResult<Self> {
        let mut m = Self { policy, files: HashSet::new(), dirs: Vec::new(), patterns: Vec::new() };
        for r in rules.iter().filter(|r| r.enabled) {
            match r.entry_type {
                RuleType::File => {
                    m.files.insert(RelPath::parse(&r.relative_path)?.key(policy));
                }
                RuleType::Directory => m.dirs.push(RelPath::parse(&r.relative_path)?),
                RuleType::NamePattern => m.patterns.push(r.relative_path.clone()),
            }
        }
        Ok(m)
    }

    /// 没有任何用户规则（仍会排除系统内部临时文件）。
    pub fn empty(policy: CasePolicy) -> Self {
        Self { policy, files: HashSet::new(), dirs: Vec::new(), patterns: Vec::new() }
    }

    /// `rel` 本身或其任一上级被排除时返回 true。
    pub fn is_excluded(&self, rel: &RelPath, is_dir: bool) -> bool {
        if rel.file_name().starts_with(INTERNAL_TMP_PREFIX) {
            return true;
        }
        if !is_dir && self.files.contains(&rel.key(self.policy)) {
            return true;
        }
        if self.dirs.iter().any(|d| rel.is_within(d, self.policy)) {
            return true;
        }
        !self.patterns.is_empty() && rel.components().any(|c| self.patterns.iter().any(|p| wildcard_match(p, c)))
    }

    /// 某条规则是否匹配 `rel`（用于统计每条规则匹配的文件数）。
    pub fn rule_matches(rule: &ExclusionRule, rel: &RelPath, is_dir: bool, policy: CasePolicy) -> bool {
        match rule.entry_type {
            RuleType::File => {
                !is_dir && RelPath::parse(&rule.relative_path).is_ok_and(|r| r.key(policy) == rel.key(policy))
            }
            RuleType::Directory => RelPath::parse(&rule.relative_path).is_ok_and(|d| rel.is_within(&d, policy)),
            RuleType::NamePattern => rel.components().any(|c| wildcard_match(&rule.relative_path, c)),
        }
    }
}

/// 规则快照：只含已启用规则，排序后序列化，用于记录到版本中并判断“规则是否变化”。
pub fn snapshot_json(rules: &[ExclusionRule], policy: CasePolicy) -> String {
    let mut enabled: Vec<&ExclusionRule> = rules.iter().filter(|r| r.enabled).collect();
    enabled.sort_by_cached_key(|r| (r.entry_type.as_str(), path_key(&r.relative_path, policy)));
    serde_json::to_string(&enabled).unwrap_or_else(|_| "[]".to_owned())
}

pub fn parse_snapshot(json: &str) -> Vec<ExclusionRule> {
    serde_json::from_str(json).unwrap_or_default()
}

pub(crate) fn load_rules(conn: &Connection, project_id: &str) -> CoreResult<Vec<ExclusionRule>> {
    let mut stmt = conn.prepare(
        "SELECT relative_path, entry_type, is_system_default, enabled FROM exclusion_rules
          WHERE project_id = ?1 ORDER BY is_system_default DESC, relative_path",
    )?;
    let rows = stmt.query_map([project_id], |r| {
        Ok(ExclusionRule {
            relative_path: r.get(0)?,
            entry_type: RuleType::parse(&r.get::<_, String>(1)?),
            is_system_default: r.get(2)?,
            enabled: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// 用完整规则集合替换项目的规则（调用方负责事务）。
pub(crate) fn replace_rules(
    conn: &Connection,
    project_id: &str,
    rules: &[ExclusionRule],
    policy: CasePolicy,
) -> CoreResult<()> {
    conn.execute("DELETE FROM exclusion_rules WHERE project_id = ?1", [project_id])?;
    let mut stmt = conn.prepare(
        "INSERT OR IGNORE INTO exclusion_rules
           (rule_id, project_id, relative_path, path_key, entry_type, is_system_default, enabled)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for r in rules {
        stmt.execute(params![
            crate::new_id(),
            project_id,
            r.relative_path,
            path_key(&r.relative_path, policy),
            r.entry_type.as_str(),
            r.is_system_default,
            r.enabled,
        ])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: CasePolicy = CasePolicy::Sensitive;

    fn rule(p: &str, t: RuleType) -> ExclusionRule {
        ExclusionRule { relative_path: p.into(), entry_type: t, is_system_default: false, enabled: true }
    }

    fn rp(s: &str) -> RelPath {
        RelPath::parse(s).unwrap()
    }

    #[test]
    fn directory_rule_covers_descendants_only_on_boundary() {
        let m = Matcher::new(&[rule("a/b", RuleType::Directory)], S).unwrap();
        assert!(m.is_excluded(&rp("a/b"), true));
        assert!(m.is_excluded(&rp("a/b/c/d.txt"), false));
        assert!(!m.is_excluded(&rp("a/b2"), true));
        assert!(!m.is_excluded(&rp("a"), true));
    }

    #[test]
    fn file_rule_matches_only_that_file() {
        let m = Matcher::new(&[rule("a/x.log", RuleType::File)], S).unwrap();
        assert!(m.is_excluded(&rp("a/x.log"), false));
        assert!(!m.is_excluded(&rp("a/x.log2"), false));
        assert!(!m.is_excluded(&rp("a/x.log/y"), false));
    }

    #[test]
    fn system_defaults_match_anywhere_and_can_be_disabled() {
        let mut rules = system_default_rules();
        let m = Matcher::new(&rules, S).unwrap();
        assert!(m.is_excluded(&rp("docs/~$报告.docx"), false));
        assert!(m.is_excluded(&rp("pics/thumbs.db"), false));
        assert!(m.is_excluded(&rp("Desktop.ini"), false));
        assert!(!m.is_excluded(&rp("docs/报告.docx"), false));
        rules[0].enabled = false;
        let m = Matcher::new(&rules, S).unwrap();
        assert!(!m.is_excluded(&rp("docs/~$报告.docx"), false));
    }

    #[test]
    fn internal_temp_files_always_excluded() {
        assert!(Matcher::empty(S).is_excluded(&rp("a/.lfvm~01J.tmp"), false));
    }

    #[test]
    fn normalize_rejects_bad_rules() {
        assert!(normalize_rule(&rule("../x", RuleType::File)).is_err());
        assert!(normalize_rule(&rule("/abs", RuleType::Directory)).is_err());
        assert!(normalize_rule(&rule("*.tmp", RuleType::NamePattern)).is_err());
        let mut sys = system_default_rules()[0].clone();
        sys.relative_path = "*.png".into();
        assert!(normalize_rule(&sys).is_err());
        assert_eq!(normalize_rule(&rule("a\\b\\", RuleType::Directory)).unwrap().relative_path, "a/b");
    }

    #[test]
    fn snapshot_is_order_independent_and_ignores_disabled() {
        let a = vec![rule("b", RuleType::File), rule("a", RuleType::File)];
        let mut b = vec![rule("a", RuleType::File), rule("b", RuleType::File), rule("c", RuleType::File)];
        b[2].enabled = false;
        assert_eq!(snapshot_json(&a, S), snapshot_json(&b, S));
        assert_eq!(parse_snapshot(&snapshot_json(&a, S)).len(), 2);
    }
}
