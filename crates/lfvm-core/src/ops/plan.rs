//! 影响清单：工作区与目标版本的对比结果（SRS 3.3.2.5 第 2～3 步、规则 R-05）。
//!
//! - 当前被排除的路径一律不改动（keep）；
//! - 工作区中多出的文件删除，缺少的新建，内容不同的替换，“文件↔文件夹”变化拆成删除 + 新建；
//! - 含有被排除内容的文件夹不删除（保留下来不影响目标结构）；
//!   但若目标要求在这里放一个文件，或目标位置被排除的内容占用，则保留排除内容与目标结构
//!   无法同时成立，在写入前中止（LFVM-AT-09）。

use std::path::Path;

use serde::Serialize;

use crate::changes::Manifest;
use crate::error::ErrorCode;
use crate::exclude::Matcher;
use crate::hash::hash_bytes;
use crate::model::EntryType;
use crate::paths::RelPath;
use crate::scan::{Scan, ScanProblem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Create,
    Replace,
    Delete,
    /// 因排除而保留不动。
    Keep,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Create => "create",
            Action::Replace => "replace",
            Action::Delete => "delete",
            Action::Keep => "keep",
        }
    }

    pub(crate) fn parse(s: &str) -> Self {
        match s {
            "create" => Action::Create,
            "replace" => Action::Replace,
            "delete" => Action::Delete,
            _ => Action::Keep,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PlanItem {
    pub path: String,
    #[serde(skip)]
    pub key: String,
    pub action: Action,
    pub entry_type: EntryType,
    /// 现有内容的摘要（替换、删除文件时）。
    pub before_hash: Option<String>,
    /// 目标内容的摘要（新建、替换文件时）。
    pub after_hash: Option<String>,
    #[cfg_attr(feature = "specta", specta(type = Option<specta_typescript::Number>))]
    pub before_size: Option<i64>,
    #[cfg_attr(feature = "specta", specta(type = Option<specta_typescript::Number>))]
    pub after_size: Option<i64>,
    /// 属于“文件↔文件夹”类型变化。
    pub type_change: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub items: Vec<PlanItem>,
    pub conflicts: Vec<ScanProblem>,
}

impl Plan {
    /// 计划的指纹：用户确认后重新计算，指纹不同说明文件夹在确认后又变了。
    pub fn fingerprint(&self) -> String {
        let mut lines: Vec<String> = self
            .items
            .iter()
            .map(|i| {
                format!(
                    "{}|{}|{}|{}",
                    i.key,
                    i.action.as_str(),
                    i.before_hash.as_deref().unwrap_or("-"),
                    i.after_hash.as_deref().unwrap_or("-")
                )
            })
            .collect();
        lines.sort();
        hash_bytes(lines.join("\n").as_bytes())
    }

    /// 执行顺序：先删文件，再由深到浅删文件夹，再由浅到深建文件夹，最后写文件。
    /// 返回（在 items 中的下标, 项）。
    pub fn ordered(&self) -> Vec<(usize, &PlanItem)> {
        let depth = |i: &PlanItem| i.path.matches('/').count();
        let rank = |i: &PlanItem| match (i.action, i.entry_type) {
            (Action::Delete, EntryType::File) => 0,
            (Action::Delete, EntryType::Directory) => 1,
            (Action::Create, EntryType::Directory) => 2,
            (Action::Create | Action::Replace, EntryType::File) => 3,
            _ => 4,
        };
        let mut v: Vec<(usize, &PlanItem)> =
            self.items.iter().enumerate().filter(|(_, i)| i.action != Action::Keep).collect();
        v.sort_by(|(_, a), (_, b)| {
            rank(a).cmp(&rank(b)).then_with(|| match rank(a) {
                1 => depth(b).cmp(&depth(a)),
                _ => depth(a).cmp(&depth(b)),
            })
        });
        v
    }

    pub fn count(&self, action: Action) -> usize {
        self.items.iter().filter(|i| i.action == action).count()
    }
}

/// 计算把工作区变成目标版本需要做的事。`current` 是按当前排除规则扫描的结果。
pub fn compute(current: &Scan, matcher: &Matcher, target: &Manifest, root: &Path) -> Plan {
    let mut plan = Plan::default();
    let policy_free_within = |inner: &RelPath, outer: &RelPath| {
        inner.as_str().len() > outer.as_str().len()
            && inner.as_str().starts_with(outer.as_str())
            && inner.as_str().as_bytes()[outer.as_str().len()] == b'/'
    };
    // 目录下是否有被排除的内容（这样的目录不能删除）
    let holds_excluded = |dir: &RelPath| current.excluded.iter().any(|e| policy_free_within(e, dir));
    let conflict = |path: &str, msg: String| ScanProblem {
        path: path.to_owned(),
        code: ErrorCode::StructureConflict,
        message: msg,
    };
    let size = |s: Option<u64>| s.map(|v| v as i64);

    for (key, t) in target {
        let Ok(rel) = RelPath::parse(&t.rel) else { continue };
        if matcher.is_excluded(&rel, t.entry_type.is_dir()) {
            plan.items.push(PlanItem {
                path: t.rel.clone(),
                key: key.clone(),
                action: Action::Keep,
                entry_type: t.entry_type,
                before_hash: None,
                after_hash: t.hash.clone(),
                before_size: None,
                after_size: size(t.size),
                type_change: false,
            });
            continue;
        }
        if let Some(bad) = rel.components().find(|c| lfvm_platform::validate_component(c).is_err()) {
            plan.conflicts.push(conflict(&t.rel, format!("名称“{bad}”在当前系统上不能使用，无法写入")));
            continue;
        }
        let create = |type_change: bool| PlanItem {
            path: t.rel.clone(),
            key: key.clone(),
            action: Action::Create,
            entry_type: t.entry_type,
            before_hash: None,
            after_hash: t.hash.clone(),
            before_size: None,
            after_size: size(t.size),
            type_change,
        };
        match current.entries.get(key) {
            None => {
                if std::fs::symlink_metadata(rel.to_path(root)).is_ok() {
                    plan.conflicts
                        .push(conflict(&t.rel, "目标版本需要在这里放置内容，但这个位置当前被排除的内容占用".into()));
                } else {
                    plan.items.push(create(false));
                }
            }
            Some(w) if w.entry_type == t.entry_type => {
                if t.entry_type == EntryType::File && w.hash != t.hash {
                    plan.items.push(PlanItem {
                        action: Action::Replace,
                        before_hash: w.hash.clone(),
                        before_size: Some(w.size as i64),
                        ..create(false)
                    });
                }
            }
            Some(w) => {
                if w.entry_type.is_dir() && holds_excluded(&w.rel) {
                    plan.conflicts.push(conflict(
                        &t.rel,
                        "目标版本在这里是一个文件，但当前同名文件夹中有被排除的内容，无法替换".into(),
                    ));
                    continue;
                }
                plan.items.push(PlanItem {
                    path: w.rel.as_str().to_owned(),
                    key: key.clone(),
                    action: Action::Delete,
                    entry_type: w.entry_type,
                    before_hash: w.hash.clone(),
                    after_hash: None,
                    before_size: (!w.entry_type.is_dir()).then_some(w.size as i64),
                    after_size: None,
                    type_change: true,
                });
                plan.items.push(create(true));
            }
        }
    }

    for (key, w) in &current.entries {
        if target.contains_key(key) {
            continue;
        }
        let keep = w.entry_type.is_dir() && holds_excluded(&w.rel);
        plan.items.push(PlanItem {
            path: w.rel.as_str().to_owned(),
            key: key.clone(),
            action: if keep { Action::Keep } else { Action::Delete },
            entry_type: w.entry_type,
            before_hash: w.hash.clone(),
            after_hash: None,
            before_size: (!w.entry_type.is_dir()).then_some(w.size as i64),
            after_size: None,
            type_change: false,
        });
    }
    // 类型变化中被删除的文件夹，其下的内容在上面已逐项列为删除
    plan.items.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.action.as_str().cmp(b.action.as_str())));
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::ManifestEntry;
    use crate::exclude::{ExclusionRule, RuleType};
    use crate::scan::WorkEntry;
    use lfvm_platform::CasePolicy;

    const S: CasePolicy = CasePolicy::Sensitive;

    fn w(p: &str, t: EntryType, h: Option<&str>) -> (String, WorkEntry) {
        (
            p.into(),
            WorkEntry {
                rel: RelPath::parse(p).unwrap(),
                entry_type: t,
                size: 1,
                mtime_ns: 0,
                file_id: None,
                hash: h.map(str::to_owned),
            },
        )
    }

    fn m(p: &str, t: EntryType, h: Option<&str>) -> (String, ManifestEntry) {
        (p.into(), ManifestEntry { rel: p.into(), entry_type: t, size: h.map(|_| 1), hash: h.map(str::to_owned) })
    }

    fn dir_rule(p: &str) -> ExclusionRule {
        ExclusionRule {
            relative_path: p.into(),
            entry_type: RuleType::Directory,
            is_system_default: false,
            enabled: true,
        }
    }

    fn actions(p: &Plan) -> Vec<(&str, Action)> {
        p.items.iter().map(|i| (i.path.as_str(), i.action)).collect()
    }

    use EntryType::{Directory as D, File as F};

    #[test]
    fn creates_replaces_deletes() {
        let scan = Scan {
            entries: [w("same", F, Some("1")), w("mod", F, Some("1")), w("extra", F, Some("1")), w("olddir", D, None)]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let target: Manifest =
            [m("same", F, Some("1")), m("mod", F, Some("2")), m("new", F, Some("3")), m("nd", D, None)]
                .into_iter()
                .collect();
        let d = tempfile::tempdir().unwrap();
        let p = compute(&scan, &Matcher::empty(S), &target, d.path());
        assert!(p.conflicts.is_empty());
        assert_eq!(
            actions(&p),
            [
                ("extra", Action::Delete),
                ("mod", Action::Replace),
                ("nd", Action::Create),
                ("new", Action::Create),
                ("olddir", Action::Delete)
            ]
        );
        let order: Vec<&str> = p.ordered().iter().map(|(_, i)| i.path.as_str()).collect();
        assert_eq!(order, ["extra", "olddir", "nd", "mod", "new"]);
    }

    #[test]
    fn excluded_target_paths_are_kept() {
        let scan = Scan::default();
        let target: Manifest = [m("cache", D, None), m("cache/x", F, Some("1"))].into_iter().collect();
        let mt = Matcher::new(&[dir_rule("cache")], S).unwrap();
        let d = tempfile::tempdir().unwrap();
        let p = compute(&scan, &mt, &target, d.path());
        assert!(p.items.iter().all(|i| i.action == Action::Keep));
    }

    /// LFVM-AT-09：排除了 a/cache，目标版本中 a 为文件，写入前报结构冲突；普通的同路径排除只保留不中止。
    #[test]
    fn excluded_content_blocks_type_change_only() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("a/cache")).unwrap();
        let mt = Matcher::new(&[dir_rule("a/cache")], S).unwrap();
        let scan = Scan {
            entries: [w("a", D, None), w("a/f", F, Some("1"))].into_iter().collect(),
            excluded: vec![RelPath::parse("a/cache").unwrap()],
            ..Default::default()
        };
        let as_file: Manifest = [m("a", F, Some("9"))].into_iter().collect();
        let p = compute(&scan, &mt, &as_file, d.path());
        assert_eq!(p.conflicts.len(), 1);
        assert_eq!(p.conflicts[0].code, ErrorCode::StructureConflict);

        // 目标中没有 a：a 中的普通文件删除，但 a 因含有排除内容而保留
        let p = compute(&scan, &mt, &Manifest::new(), d.path());
        assert!(p.conflicts.is_empty());
        assert_eq!(actions(&p), [("a", Action::Keep), ("a/f", Action::Delete)]);
    }

    #[test]
    fn type_change_file_to_dir() {
        let scan = Scan { entries: [w("t", F, Some("1"))].into_iter().collect(), ..Default::default() };
        let target: Manifest = [m("t", D, None), m("t/x", F, Some("2"))].into_iter().collect();
        let d = tempfile::tempdir().unwrap();
        let p = compute(&scan, &Matcher::empty(S), &target, d.path());
        let order: Vec<(&str, Action, EntryType)> =
            p.ordered().iter().map(|(_, i)| (i.path.as_str(), i.action, i.entry_type)).collect();
        assert_eq!(order, [("t", Action::Delete, F), ("t", Action::Create, D), ("t/x", Action::Create, F)]);
    }

    #[test]
    fn fingerprint_changes_with_content() {
        let scan = Scan { entries: [w("a", F, Some("1"))].into_iter().collect(), ..Default::default() };
        let d = tempfile::tempdir().unwrap();
        let t1: Manifest = [m("a", F, Some("2"))].into_iter().collect();
        let t2: Manifest = [m("a", F, Some("3"))].into_iter().collect();
        let a = compute(&scan, &Matcher::empty(S), &t1, d.path()).fingerprint();
        assert_eq!(a, compute(&scan, &Matcher::empty(S), &t1, d.path()).fingerprint());
        assert_ne!(a, compute(&scan, &Matcher::empty(S), &t2, d.path()).fingerprint());
    }
}
