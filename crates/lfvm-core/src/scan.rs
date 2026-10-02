//! 扫描工作文件夹（SRS 3.3.1.3、2.3.1）。
//!
//! - 不跟随链接对象；未排除的链接、云占位文件等记为“问题”，保存前须由用户排除；
//! - 已排除的文件夹不进入，只记录其路径；
//! - 大小、修改时间、文件标识都未变的文件沿用缓存中的摘要，其余文件并行计算摘要（LFVM-P-05）。

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use lfvm_platform::{CasePolicy, EntryKind};
use rayon::prelude::*;
use rusqlite::{Connection, params};
use serde::Serialize;

use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::exclude::Matcher;
use crate::hash::hash_file;
use crate::model::EntryType;
use crate::paths::RelPath;
use crate::progress::{Progress, Stage};

/// 工作区中的一个条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkEntry {
    pub rel: RelPath,
    pub entry_type: EntryType,
    pub size: u64,
    pub mtime_ns: i64,
    pub file_id: Option<String>,
    /// 文件内容摘要；目录为 None。
    pub hash: Option<String>,
}

/// 扫描时遇到、需要用户处理的条目。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ScanProblem {
    pub path: String,
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct Scan {
    /// 以 path_key 为键，有序。
    pub entries: BTreeMap<String, WorkEntry>,
    /// 按规则跳过的路径（被排除的文件夹只记录文件夹本身）。
    pub excluded: Vec<RelPath>,
    pub problems: Vec<ScanProblem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntry {
    pub size: u64,
    pub mtime_ns: i64,
    pub file_id: Option<String>,
    pub hash: String,
}

pub type ScanCache = HashMap<String, CacheEntry>;

/// 扫描 `root`。`cache` 用于跳过未变化文件的摘要计算。
pub fn scan(
    root: &Path,
    matcher: &Matcher,
    policy: CasePolicy,
    cache: &ScanCache,
    progress: &dyn Progress,
) -> CoreResult<Scan> {
    let mut out = Scan::default();
    let mut stack: Vec<Option<RelPath>> = vec![None];
    let mut listed = 0u64;

    while let Some(dir_rel) = stack.pop() {
        progress.check()?;
        let dir_abs = dir_rel.as_ref().map_or_else(|| root.to_path_buf(), |r| r.to_path(root));
        let rd = match std::fs::read_dir(&dir_abs) {
            Ok(rd) => rd,
            Err(e) if dir_rel.is_none() => return Err(CoreError::io(e, &dir_abs)),
            Err(e) => {
                let err = CoreError::io(e, &dir_abs);
                out.problems.push(problem(dir_rel.as_ref().map_or("", |r| r.as_str()), err.code, err.message));
                continue;
            }
        };
        for item in rd {
            let item = item.map_err(|e| CoreError::io(e, &dir_abs))?;
            let os_name = item.file_name();
            let display = os_name.to_string_lossy().into_owned();
            let rel_display = match &dir_rel {
                Some(p) => format!("{p}/{display}"),
                None => display.clone(),
            };
            let Some(name) = os_name.to_str() else {
                out.problems.push(problem(&rel_display, ErrorCode::InvalidPath, "名称中含有无法处理的字符"));
                continue;
            };
            let rel = match RelPath::child(dir_rel.as_ref(), name) {
                Ok(r) => r,
                Err(e) => {
                    out.problems.push(problem(&rel_display, e.code, e.message));
                    continue;
                }
            };
            let abs = item.path();
            let meta = match std::fs::symlink_metadata(&abs) {
                Ok(m) => m,
                Err(e) => {
                    let err = CoreError::io(e, &abs);
                    out.problems.push(problem(rel.as_str(), err.code, err.message));
                    continue;
                }
            };
            let kind = lfvm_platform::classify(&meta);
            let is_dir = kind == EntryKind::Dir;
            if matcher.is_excluded(&rel, is_dir) {
                if !rel.file_name().starts_with(crate::exclude::INTERNAL_TMP_PREFIX) {
                    out.excluded.push(rel);
                }
                continue;
            }
            listed += 1;
            if listed.is_multiple_of(256) {
                progress.report(Stage::Listing, listed, 0);
            }
            match kind {
                EntryKind::Dir => {
                    out.entries.insert(
                        rel.key(policy),
                        WorkEntry {
                            rel: rel.clone(),
                            entry_type: EntryType::Directory,
                            size: 0,
                            mtime_ns: 0,
                            file_id: None,
                            hash: None,
                        },
                    );
                    stack.push(Some(rel));
                }
                EntryKind::File => {
                    out.entries.insert(
                        rel.key(policy),
                        WorkEntry {
                            rel,
                            entry_type: EntryType::File,
                            size: meta.len(),
                            mtime_ns: mtime_ns(&meta),
                            file_id: lfvm_platform::file_id(&meta).map(|(d, i)| format!("{d}:{i}")),
                            hash: None,
                        },
                    );
                }
                EntryKind::Link => out.problems.push(problem(
                    rel.as_str(),
                    ErrorCode::LinkNotFollowed,
                    "这是一个快捷链接（符号链接或目录联接），系统不会跟随。请把它设为排除项后再保存",
                )),
                EntryKind::CloudPlaceholder => out.problems.push(problem(
                    rel.as_str(),
                    ErrorCode::CloudPlaceholder,
                    "这个文件只存在于云端，请先下载到本机，或把它设为排除项",
                )),
                EntryKind::Other => out.problems.push(problem(
                    rel.as_str(),
                    ErrorCode::InvalidPath,
                    "这不是普通文件或文件夹，请把它设为排除项",
                )),
            }
        }
    }

    hash_entries(root, &mut out, policy, cache, progress)?;
    out.excluded.sort();
    Ok(out)
}

fn hash_entries(
    root: &Path,
    out: &mut Scan,
    policy: CasePolicy,
    cache: &ScanCache,
    progress: &dyn Progress,
) -> CoreResult<()> {
    let mut todo: Vec<&mut WorkEntry> = Vec::new();
    for e in out.entries.values_mut().filter(|e| e.entry_type == EntryType::File) {
        match cache.get(&e.rel.key(policy)) {
            Some(c) if c.size == e.size && c.mtime_ns == e.mtime_ns && c.file_id == e.file_id => {
                e.hash = Some(c.hash.clone());
            }
            _ => todo.push(e),
        }
    }
    let total_bytes: u64 = todo.iter().map(|e| e.size).sum();
    let done = AtomicU64::new(0);
    progress.report(Stage::Hashing, 0, total_bytes);
    let failures: Vec<ScanProblem> = todo
        .par_iter_mut()
        .filter_map(|e| {
            if progress.is_cancelled() {
                return None;
            }
            let abs = e.rel.to_path(root);
            match hash_file(&abs) {
                Ok((h, size)) => {
                    e.hash = Some(h);
                    e.size = size;
                    let d = done.fetch_add(size, Ordering::Relaxed) + size;
                    progress.report(Stage::Hashing, d, total_bytes);
                    None
                }
                Err(err) => {
                    let err = CoreError::io(err, &abs);
                    Some(problem(e.rel.as_str(), err.code, err.message))
                }
            }
        })
        .collect();
    progress.check()?;
    if !failures.is_empty() {
        let bad: std::collections::HashSet<String> = failures.iter().map(|p| p.path.clone()).collect();
        out.entries.retain(|_, e| !bad.contains(e.rel.as_str()));
        out.problems.extend(failures);
    }
    Ok(())
}

fn problem(path: &str, code: ErrorCode, message: impl Into<String>) -> ScanProblem {
    ScanProblem { path: path.to_owned(), code, message: message.into() }
}

fn mtime_ns(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos().min(i64::MAX as u128) as i64)
}

pub(crate) fn load_cache(conn: &Connection, project_id: &str) -> CoreResult<ScanCache> {
    let mut stmt =
        conn.prepare("SELECT path_key, size, mtime_ns, file_id, content_hash FROM scan_cache WHERE project_id = ?1")?;
    let rows = stmt.query_map([project_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            CacheEntry { size: r.get::<_, i64>(1)? as u64, mtime_ns: r.get(2)?, file_id: r.get(3)?, hash: r.get(4)? },
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// 用本次扫描结果替换缓存。
pub(crate) fn store_cache(conn: &mut Connection, project_id: &str, scan: &Scan, policy: CasePolicy) -> CoreResult<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM scan_cache WHERE project_id = ?1", [project_id])?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO scan_cache (project_id, path_key, size, mtime_ns, file_id, content_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for e in scan.entries.values() {
            if let Some(h) = &e.hash {
                stmt.execute(params![project_id, e.rel.key(policy), e.size as i64, e.mtime_ns, e.file_id, h])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exclude::{ExclusionRule, RuleType, system_default_rules};
    use crate::hash::hash_bytes;
    use crate::progress::NoProgress;

    const S: CasePolicy = CasePolicy::Sensitive;

    fn setup() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir_all(r.join("docs/empty")).unwrap();
        std::fs::create_dir_all(r.join("cache/deep")).unwrap();
        std::fs::write(r.join("docs/a.txt"), b"aaa").unwrap();
        std::fs::write(r.join("docs/~$a.docx"), b"lock").unwrap();
        std::fs::write(r.join("zero.bin"), b"").unwrap();
        std::fs::write(r.join("cache/deep/x"), b"x").unwrap();
        d
    }

    #[test]
    fn scans_files_dirs_and_respects_exclusions() {
        let d = setup();
        let mut rules = system_default_rules();
        rules.push(ExclusionRule {
            relative_path: "cache".into(),
            entry_type: RuleType::Directory,
            is_system_default: false,
            enabled: true,
        });
        let m = Matcher::new(&rules, S).unwrap();
        let s = scan(d.path(), &m, S, &ScanCache::new(), &NoProgress).unwrap();
        let keys: Vec<&str> = s.entries.keys().map(String::as_str).collect();
        assert_eq!(keys, ["docs", "docs/a.txt", "docs/empty", "zero.bin"]);
        assert_eq!(s.entries["docs/a.txt"].hash.as_deref(), Some(hash_bytes(b"aaa").as_str()));
        assert_eq!(s.entries["zero.bin"].hash.as_deref(), Some(hash_bytes(b"").as_str()));
        let ex: Vec<&str> = s.excluded.iter().map(RelPath::as_str).collect();
        assert_eq!(ex, ["cache", "docs/~$a.docx"]);
        assert!(s.problems.is_empty());
    }

    #[test]
    fn uses_cache_only_when_metadata_unchanged() {
        let d = setup();
        let m = Matcher::empty(S);
        let first = scan(d.path(), &m, S, &ScanCache::new(), &NoProgress).unwrap();
        let mut cache: ScanCache = first
            .entries
            .iter()
            .filter_map(|(k, e)| {
                e.hash.as_ref().map(|h| {
                    (
                        k.clone(),
                        CacheEntry { size: e.size, mtime_ns: e.mtime_ns, file_id: e.file_id.clone(), hash: h.clone() },
                    )
                })
            })
            .collect();
        // 伪造缓存中的摘要：元数据一致时应沿用缓存
        cache.get_mut("docs/a.txt").unwrap().hash = "f".repeat(64);
        let again = scan(d.path(), &m, S, &cache, &NoProgress).unwrap();
        assert_eq!(again.entries["docs/a.txt"].hash.as_deref(), Some("f".repeat(64).as_str()));
        // 元数据变化时重新计算
        cache.get_mut("docs/a.txt").unwrap().size = 999;
        let third = scan(d.path(), &m, S, &cache, &NoProgress).unwrap();
        assert_eq!(third.entries["docs/a.txt"].hash.as_deref(), Some(hash_bytes(b"aaa").as_str()));
    }

    #[cfg(unix)]
    #[test]
    fn reports_unexcluded_links_without_following() {
        let d = setup();
        std::os::unix::fs::symlink(d.path().join("docs"), d.path().join("link")).unwrap();
        let s = scan(d.path(), &Matcher::empty(S), S, &ScanCache::new(), &NoProgress).unwrap();
        assert_eq!(s.problems.len(), 1);
        assert_eq!(s.problems[0].code, ErrorCode::LinkNotFollowed);
        assert!(!s.entries.contains_key("link"));
        assert!(!s.entries.keys().any(|k| k.starts_with("link/")));
    }

    #[test]
    fn cancellation_stops_scan() {
        struct Cancelled;
        impl Progress for Cancelled {
            fn report(&self, _: Stage, _: u64, _: u64) {}
            fn is_cancelled(&self) -> bool {
                true
            }
        }
        let d = setup();
        let err = scan(d.path(), &Matcher::empty(S), S, &ScanCache::new(), &Cancelled).unwrap_err();
        assert_eq!(err.code, ErrorCode::Cancelled);
    }
}
