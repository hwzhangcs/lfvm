//! 当前变化：工作区相对比较基准的新增、修改、删除（SRS 3.3.1.3，规则 R-01）。

use std::collections::BTreeMap;

use rusqlite::Connection;
use serde::Serialize;

use crate::error::CoreResult;
use crate::exclude::Matcher;
use crate::model::EntryType;
use crate::paths::RelPath;
use crate::scan::{Scan, ScanProblem};

/// 版本清单中的一项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    pub rel: String,
    pub entry_type: EntryType,
    pub size: Option<u64>,
    pub hash: Option<String>,
}

/// 以 path_key 为键的版本清单。
pub type Manifest = BTreeMap<String, ManifestEntry>;

pub(crate) fn load_manifest(conn: &Connection, version_id: &str) -> CoreResult<Manifest> {
    let mut stmt = conn.prepare(
        "SELECT path_key, relative_path, entry_type, size, content_hash FROM version_files WHERE version_id = ?1",
    )?;
    let rows = stmt.query_map([version_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            ManifestEntry {
                rel: r.get(1)?,
                entry_type: EntryType::parse(&r.get::<_, String>(2)?),
                size: r.get::<_, Option<i64>>(3)?.map(|s| s as u64),
                hash: r.get(4)?,
            },
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    /// 同一路径在文件与文件夹之间变化。
    TypeChanged,
    /// 基准中有、但按当前排除规则不再纳入（不计为删除）。
    ExcludedNow,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ChangeItem {
    pub path: String,
    pub kind: ChangeKind,
    /// 当前的类型；已删除或不再纳入时为基准中的类型。
    pub entry_type: EntryType,
    /// 文件大小（字节）；文件夹为 null。
    #[cfg_attr(feature = "specta", specta(type = Option<specta_typescript::Number>))]
    pub size: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ChangeCounts {
    /// 新增、修改、删除的文件数（不含文件夹）。
    pub added: u32,
    pub modified: u32,
    pub deleted: u32,
    pub dirs_added: u32,
    pub dirs_deleted: u32,
    pub excluded_now: u32,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ChangeSet {
    /// 比较基准；尚无版本时为 null，此时全部文件显示为新增。
    pub baseline_version_id: Option<String>,
    pub items: Vec<ChangeItem>,
    pub counts: ChangeCounts,
    /// 排除规则相对基准版本保存时发生了变化。
    pub rules_changed: bool,
    /// 需要用户处理的条目（链接、云占位、无法读取等）；存在时不能保存。
    pub problems: Vec<ScanProblem>,
    pub file_count: u32,
    pub directory_count: u32,
    /// 有可保存的变化（尚无版本时总为 true，允许保存空文件夹）。
    pub has_changes: bool,
}

impl ChangeSet {
    pub fn can_save(&self) -> bool {
        self.has_changes && self.problems.is_empty()
    }
}

pub(crate) fn diff(
    baseline_id: Option<&str>,
    baseline: &Manifest,
    scan: &Scan,
    matcher: &Matcher,
    rules_changed: bool,
) -> ChangeSet {
    let mut items = Vec::new();
    let mut c = ChangeCounts::default();
    let size_of = |s: u64| Some(s as i64);

    for (key, cur) in &scan.entries {
        match baseline.get(key) {
            None => {
                if cur.entry_type.is_dir() {
                    c.dirs_added += 1;
                } else {
                    c.added += 1;
                }
                items.push(ChangeItem {
                    path: cur.rel.as_str().to_owned(),
                    kind: ChangeKind::Added,
                    entry_type: cur.entry_type,
                    size: (!cur.entry_type.is_dir()).then_some(cur.size as i64),
                });
            }
            Some(old) if old.entry_type != cur.entry_type => {
                if old.entry_type.is_dir() {
                    c.dirs_deleted += 1;
                    c.added += 1;
                } else {
                    c.deleted += 1;
                    c.dirs_added += 1;
                }
                items.push(ChangeItem {
                    path: cur.rel.as_str().to_owned(),
                    kind: ChangeKind::TypeChanged,
                    entry_type: cur.entry_type,
                    size: (!cur.entry_type.is_dir()).then_some(cur.size as i64),
                });
            }
            Some(old) if !cur.entry_type.is_dir() && old.hash != cur.hash => {
                c.modified += 1;
                items.push(ChangeItem {
                    path: cur.rel.as_str().to_owned(),
                    kind: ChangeKind::Modified,
                    entry_type: EntryType::File,
                    size: size_of(cur.size),
                });
            }
            Some(_) => {}
        }
    }

    for (key, old) in baseline {
        if scan.entries.contains_key(key) {
            continue;
        }
        let excluded = RelPath::parse(&old.rel).is_ok_and(|r| matcher.is_excluded(&r, old.entry_type.is_dir()));
        let kind = if excluded {
            c.excluded_now += 1;
            ChangeKind::ExcludedNow
        } else {
            if old.entry_type.is_dir() {
                c.dirs_deleted += 1;
            } else {
                c.deleted += 1;
            }
            ChangeKind::Deleted
        };
        items.push(ChangeItem {
            path: old.rel.clone(),
            kind,
            entry_type: old.entry_type,
            size: old.size.and_then(size_of),
        });
    }
    items.sort_by(|a, b| a.path.cmp(&b.path));

    let file_count = scan.entries.values().filter(|e| !e.entry_type.is_dir()).count() as u32;
    let directory_count = scan.entries.len() as u32 - file_count;
    ChangeSet {
        baseline_version_id: baseline_id.map(str::to_owned),
        has_changes: baseline_id.is_none() || !items.is_empty() || rules_changed,
        items,
        counts: c,
        rules_changed,
        problems: scan.problems.clone(),
        file_count,
        directory_count,
    }
}

/// 把扫描结果转换为清单形式（扫描结果与清单使用同一套 path_key）。
#[cfg(test)]
pub(crate) fn manifest_from_scan(scan: &Scan) -> Manifest {
    scan.entries
        .iter()
        .map(|(k, e)| {
            (
                k.clone(),
                ManifestEntry {
                    rel: e.rel.as_str().to_owned(),
                    entry_type: e.entry_type,
                    size: (!e.entry_type.is_dir()).then_some(e.size),
                    hash: e.hash.clone(),
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lfvm_platform::CasePolicy;
    use crate::exclude::{ExclusionRule, RuleType};
    use crate::scan::WorkEntry;

    const S: CasePolicy = CasePolicy::Sensitive;

    fn file(p: &str, h: &str) -> (String, WorkEntry) {
        (
            p.into(),
            WorkEntry {
                rel: RelPath::parse(p).unwrap(),
                entry_type: EntryType::File,
                size: 1,
                mtime_ns: 0,
                file_id: None,
                hash: Some(h.into()),
            },
        )
    }

    fn dir(p: &str) -> (String, WorkEntry) {
        (
            p.into(),
            WorkEntry {
                rel: RelPath::parse(p).unwrap(),
                entry_type: EntryType::Directory,
                size: 0,
                mtime_ns: 0,
                file_id: None,
                hash: None,
            },
        )
    }

    fn scan_of(entries: Vec<(String, WorkEntry)>) -> Scan {
        Scan { entries: entries.into_iter().collect(), ..Default::default() }
    }

    #[test]
    fn first_version_everything_added() {
        let s = scan_of(vec![dir("d"), file("d/a", "1")]);
        let cs = diff(None, &Manifest::new(), &s, &Matcher::empty(S), false);
        assert!(cs.has_changes);
        assert_eq!((cs.counts.added, cs.counts.dirs_added), (1, 1));
    }

    #[test]
    fn empty_folder_first_version_is_saveable() {
        let cs = diff(None, &Manifest::new(), &Scan::default(), &Matcher::empty(S), false);
        assert!(cs.can_save());
    }

    #[test]
    fn classifies_changes_against_baseline() {
        let base = manifest_from_scan(
            &scan_of(vec![file("same", "1"), file("mod", "1"), file("gone", "1"), file("cache/x", "1"), dir("t")]),
        );
        let now = scan_of(vec![file("same", "1"), file("mod", "2"), file("new", "1"), file("t", "9")]);
        let m = Matcher::new(
            &[ExclusionRule {
                relative_path: "cache".into(),
                entry_type: RuleType::Directory,
                is_system_default: false,
                enabled: true,
            }],
            S,
        )
        .unwrap();
        let cs = diff(Some("v1"), &base, &now, &m, true);
        let kinds: Vec<(&str, ChangeKind)> = cs.items.iter().map(|i| (i.path.as_str(), i.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("cache/x", ChangeKind::ExcludedNow),
                ("gone", ChangeKind::Deleted),
                ("mod", ChangeKind::Modified),
                ("new", ChangeKind::Added),
                ("t", ChangeKind::TypeChanged),
            ]
        );
        let c = &cs.counts;
        assert_eq!((c.added, c.modified, c.deleted, c.dirs_deleted, c.excluded_now), (2, 1, 1, 1, 1));
    }

    #[test]
    fn no_changes_when_identical_and_rules_same() {
        let s = scan_of(vec![file("a", "1")]);
        let base = manifest_from_scan(&s);
        let cs = diff(Some("v1"), &base, &s, &Matcher::empty(S), false);
        assert!(!cs.has_changes);
        assert!(diff(Some("v1"), &base, &s, &Matcher::empty(S), true).has_changes);
    }
}
