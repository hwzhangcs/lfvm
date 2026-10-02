//! 查看存储占用并清理（SRS 3.3.5.1、规则 R-03、LFVM-P-04）。
//!
//! - 占用分别统计历史版本内容、安全备份、数据库与日志、缩略图缓存、待回收内容；共享内容只计一次；
//! - 历史版本采用“清理内容、保留占位”：删除其文件清单引用，保留编号、父子关系和摘要；
//! - 可清理的版本：不是任何方案的起点或末端、不是默认历史末端、没有正在被读取、
//!   不关联未处置的未完成操作；有子版本的版本也可以清理（每个版本各自保存完整清单）；
//! - 安全备份：没有关联未处置的未完成操作即可清理；
//! - 方案：非活动方案可标记为“已清理”，不再保护其起点和末端，名称可被新方案使用；
//! - 回收不再被任何版本或备份引用的内容；删除失败的记为“待回收”，下次打开存储管理时重试。

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::ops::{BackupStatus, NewOp, OpStatus, OpType, finish_op, insert_op};
use crate::project::{VersionBrief, load_project};
use crate::store::ObjectStore;
use crate::{Core, now_ms};

#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct StorageUsage {
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub version_bytes: i64,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub backup_bytes: i64,
    /// 数据库与日志（全部项目共用）。
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub metadata_bytes: i64,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub cache_bytes: i64,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub pending_gc_bytes: i64,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub total_bytes: i64,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct VersionUsage {
    pub version: VersionBrief,
    pub scheme_name: Option<String>,
    pub cleared: bool,
    /// 清单中全部文件的大小之和。
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub total_bytes: i64,
    /// 预计可释放：只被这个版本引用的内容。
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub exclusive_bytes: i64,
    /// 不能清理的原因；null 表示可以清理。
    pub protected: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BackupUsage {
    pub backup_id: String,
    pub reason: String,
    pub status: BackupStatus,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub created_at: i64,
    pub file_count: u32,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub total_bytes: i64,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub exclusive_bytes: i64,
    pub protected: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SchemeUsage {
    pub scheme_id: String,
    pub name: String,
    pub protected: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct StorageReport {
    pub usage: StorageUsage,
    pub versions: Vec<VersionUsage>,
    pub backups: Vec<BackupUsage>,
    pub schemes: Vec<SchemeUsage>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ClearRequest {
    pub request_id: String,
    pub versions: Vec<String>,
    pub backups: Vec<String>,
    pub schemes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ClearItem {
    pub label: String,
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ClearResult {
    pub items: Vec<ClearItem>,
    /// 实际释放的字节数（按真实删除结果统计）。
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub freed_bytes: i64,
    /// 未能删除、记为“待回收”的字节数。
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub pending_gc_bytes: i64,
}

fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            match e.metadata() {
                Ok(m) if m.is_dir() => stack.push(e.path()),
                Ok(m) => total += m.len(),
                Err(_) => {}
            }
        }
    }
    total
}

/// 引用关系：每个内容对象被哪些来源（版本或备份）引用。
struct Refs {
    sizes: HashMap<String, i64>,
    by_source: HashMap<String, HashSet<String>>,
    count: HashMap<String, usize>,
}

impl Core {
    fn load_refs(&self, project_id: &str) -> CoreResult<Refs> {
        let db = self.db();
        let mut sizes = HashMap::new();
        let mut stmt = db.prepare("SELECT content_hash, byte_size FROM content_objects WHERE project_id = ?1 AND state = 'ready'")?;
        for row in stmt.query_map([project_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
            let (h, s) = row?;
            sizes.insert(h, s);
        }
        let mut by_source: HashMap<String, HashSet<String>> = HashMap::new();
        let mut stmt = db.prepare(
            "SELECT 'v:' || f.version_id, f.content_hash FROM version_files f JOIN versions v ON v.version_id = f.version_id
              WHERE v.project_id = ?1 AND f.content_hash IS NOT NULL
             UNION ALL
             SELECT 'b:' || f.backup_id, f.content_hash FROM backup_files f JOIN safety_backups b ON b.backup_id = f.backup_id
              WHERE b.project_id = ?1 AND f.content_hash IS NOT NULL",
        )?;
        for row in stmt.query_map([project_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (src, h) = row?;
            by_source.entry(src).or_default().insert(h);
        }
        let mut count: HashMap<String, usize> = HashMap::new();
        for hashes in by_source.values() {
            for h in hashes {
                *count.entry(h.clone()).or_insert(0) += 1;
            }
        }
        Ok(Refs { sizes, by_source, count })
    }

    /// 存储占用与可清理项（SRS 3.3.5.1 第 1、2 步）。顺带重试回收“待回收”的内容。
    pub fn storage_report(&self, project_id: &str) -> CoreResult<StorageReport> {
        self.retry_pending_gc(project_id)?;
        let refs = self.load_refs(project_id)?;
        let sum = |hs: &HashSet<String>| hs.iter().filter_map(|h| refs.sizes.get(h)).sum::<i64>();
        let exclusive =
            |hs: &HashSet<String>| hs.iter().filter(|h| refs.count.get(*h) == Some(&1)).filter_map(|h| refs.sizes.get(h)).sum::<i64>();

        let db = self.db();
        let project = load_project(&db, project_id)?;
        // 受保护的版本及原因
        let mut protected: HashMap<String, String> = HashMap::new();
        if let Some(h) = &project.default_head {
            protected.insert(h.clone(), "默认历史的最新版本".into());
        }
        let mut stmt = db.prepare("SELECT name, base_version_id, head_version_id FROM schemes WHERE project_id = ?1 AND state = 'active'")?;
        for row in stmt.query_map([project_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))? {
            let (name, base, head) = row?;
            protected.entry(head).or_insert(format!("方案“{name}”的最新版本"));
            protected.entry(base).or_insert(format!("方案“{name}”的起点"));
        }
        let mut stmt = db.prepare(
            "SELECT resolved_version_id FROM operations WHERE project_id = ?1 AND status = 'incomplete' AND resolution = 'open'
               AND resolved_version_id IS NOT NULL",
        )?;
        for row in stmt.query_map([project_id], |r| r.get::<_, String>(0))? {
            protected.entry(row?).or_insert("关联未处理的未完成操作".into());
        }

        let mut stmt = db.prepare(
            "SELECT v.version_id, v.seq, v.name, v.note, v.created_at, v.payload_state, s.name
               FROM versions v LEFT JOIN schemes s ON s.scheme_id = v.origin_scheme_id
              WHERE v.project_id = ?1 ORDER BY v.seq DESC",
        )?;
        let empty = HashSet::new();
        let versions = stmt
            .query_map([project_id], |r| {
                Ok((
                    VersionBrief { version_id: r.get(0)?, seq: r.get(1)?, name: r.get(2)?, note: r.get(3)?, created_at: r.get(4)? },
                    r.get::<_, String>(5)? == "cleared",
                    r.get::<_, Option<String>>(6)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|(version, cleared, scheme_name)| {
                let hs = refs.by_source.get(&format!("v:{}", version.version_id)).unwrap_or(&empty);
                let reason = if cleared {
                    Some("内容已清理".to_owned())
                } else if let Some(r) = protected.get(&version.version_id) {
                    Some(r.clone())
                } else if self.is_leased(&version.version_id) {
                    Some("正在被预览、比较、展开或导出".to_owned())
                } else {
                    None
                };
                VersionUsage { total_bytes: sum(hs), exclusive_bytes: exclusive(hs), protected: reason, version, scheme_name, cleared }
            })
            .collect();

        let mut stmt = db.prepare(
            "SELECT b.backup_id, b.reason, b.status, b.created_at, b.expected_file_count,
                    EXISTS (SELECT 1 FROM operations o WHERE o.operation_id = b.operation_id AND o.status = 'incomplete' AND o.resolution = 'open')
               FROM safety_backups b WHERE b.project_id = ?1 AND b.status != 'cleared' ORDER BY b.created_at DESC",
        )?;
        let backups = stmt
            .query_map([project_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, u32>(4)?,
                    r.get::<_, bool>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|(id, reason, status, created_at, n, open)| {
                let hs = refs.by_source.get(&format!("b:{id}")).unwrap_or(&empty);
                let status = match status.as_str() {
                    "ready" => BackupStatus::Ready,
                    "failed" => BackupStatus::Failed,
                    "cleared" => BackupStatus::Cleared,
                    _ => BackupStatus::Creating,
                };
                BackupUsage {
                    protected: if open {
                        Some("关联未处理的未完成操作".into())
                    } else if status == BackupStatus::Creating {
                        Some("正在创建".into())
                    } else {
                        None
                    },
                    total_bytes: sum(hs),
                    exclusive_bytes: exclusive(hs),
                    backup_id: id,
                    reason,
                    status,
                    created_at,
                    file_count: n,
                }
            })
            .collect();

        let mut stmt = db.prepare("SELECT scheme_id, name FROM schemes WHERE project_id = ?1 AND state = 'active' ORDER BY created_at")?;
        let schemes = stmt
            .query_map([project_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|(id, name)| SchemeUsage {
                protected: (project.active_scheme_id.as_deref() == Some(id.as_str()))
                    .then(|| "当前所在的方案，需先切换到其他方案或默认历史".to_owned()),
                scheme_id: id,
                name,
            })
            .collect();

        // 占用：版本内容与备份共享的内容只计一次（计入版本）
        let mut in_versions = HashSet::new();
        let mut in_backups = HashSet::new();
        for (src, hs) in &refs.by_source {
            let target = if src.starts_with("v:") { &mut in_versions } else { &mut in_backups };
            target.extend(hs.iter().cloned());
        }
        let version_bytes = sum(&in_versions);
        let backup_bytes = in_backups.difference(&in_versions).filter_map(|h| refs.sizes.get(h)).sum::<i64>();
        let pending_gc_bytes: i64 = db.query_row(
            "SELECT coalesce(sum(byte_size), 0) FROM content_objects WHERE project_id = ?1 AND state = 'pending_gc'",
            [project_id],
            |r| r.get(0),
        )?;
        let metadata_bytes = ["lfvm.db", "lfvm.db-wal", "lfvm.db-shm"]
            .iter()
            .filter_map(|f| std::fs::metadata(self.data_dir().join(f)).ok())
            .map(|m| m.len())
            .sum::<u64>()
            + dir_size(&self.data_dir().join("logs"));
        let cache_bytes = dir_size(&self.project_store_dir(project_id).join("thumbs"));
        let usage = StorageUsage {
            version_bytes,
            backup_bytes,
            metadata_bytes: metadata_bytes as i64,
            cache_bytes: cache_bytes as i64,
            pending_gc_bytes,
            total_bytes: version_bytes + backup_bytes + metadata_bytes as i64 + cache_bytes as i64 + pending_gc_bytes,
        };
        Ok(StorageReport { usage, versions, backups, schemes })
    }

    /// 清理所选的版本内容、安全备份和方案（SRS 3.3.5.1 第 3～5 步）。每项单独判断，逐项报告结果。
    pub fn clear_storage(&self, project_id: &str, req: &ClearRequest) -> CoreResult<ClearResult> {
        let _guard = self.begin_write(project_id)?;
        let report = self.storage_report(project_id)?;
        let op_id = {
            let db = self.db();
            match insert_op(
                &db,
                &NewOp {
                    project_id,
                    request_id: &req.request_id,
                    op_type: OpType::Clear,
                    target_ref: crate::ops::restore::TargetRef { label: "清理历史".into(), scheme: None, path: None }.to_json(),
                    resolved_version_id: None,
                    retry_of: None,
                },
            )? {
                Ok(id) => id,
                Err(_) => return Err(CoreError::new(ErrorCode::Busy, "这次清理请求已经处理过")),
            }
        };
        let mut items = Vec::new();
        let mut touched: HashSet<String> = HashSet::new();
        let mut db = self.db();
        let tx = db.transaction()?;
        let now = now_ms();

        // 方案先清理：之后其起点和末端不再受保护
        let mut released: HashSet<String> = HashSet::new();
        for id in &req.schemes {
            let Some(s) = report.schemes.iter().find(|s| &s.scheme_id == id) else {
                items.push(ClearItem { label: "方案".into(), ok: false, message: "找不到这个方案".into() });
                continue;
            };
            let label = format!("方案“{}”", s.name);
            if let Some(why) = &s.protected {
                items.push(ClearItem { label, ok: false, message: why.clone() });
                continue;
            }
            let (base, head): (String, String) = tx.query_row(
                "SELECT base_version_id, head_version_id FROM schemes WHERE scheme_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            tx.execute("UPDATE schemes SET state = 'cleared' WHERE scheme_id = ?1", [id])?;
            released.insert(base);
            released.insert(head);
            items.push(ClearItem { label, ok: true, message: "已清理，名称可被新方案使用".into() });
        }
        let still_protected = |v: &VersionUsage, tx: &rusqlite::Transaction<'_>| -> CoreResult<Option<String>> {
            match &v.protected {
                Some(why) if released.contains(&v.version.version_id) && why.starts_with("方案") => {
                    // 刚清理的方案不再保护它；但可能仍是其他方案的起点或末端
                    let other: Option<String> = tx
                        .query_row(
                            "SELECT name FROM schemes WHERE project_id = ?1 AND state = 'active'
                               AND (base_version_id = ?2 OR head_version_id = ?2) LIMIT 1",
                            params![project_id, v.version.version_id],
                            |r| r.get(0),
                        )
                        .ok();
                    Ok(other.map(|n| format!("方案“{n}”的起点或最新版本")))
                }
                other => Ok(other.clone()),
            }
        };
        for id in &req.versions {
            let Some(v) = report.versions.iter().find(|v| &v.version.version_id == id) else {
                items.push(ClearItem { label: "版本".into(), ok: false, message: "找不到这个版本".into() });
                continue;
            };
            let label = format!("V{}", v.version.seq);
            if let Some(why) = still_protected(v, &tx)? {
                items.push(ClearItem { label, ok: false, message: why });
                continue;
            }
            if self.is_leased(id) {
                items.push(ClearItem { label, ok: false, message: "正在被预览、比较、展开或导出".into() });
                continue;
            }
            touched.extend(self.hashes_of(&tx, "version_files", "version_id", id)?);
            tx.execute("DELETE FROM version_files WHERE version_id = ?1", [id])?;
            tx.execute("UPDATE versions SET payload_state = 'cleared', cleared_at = ?2 WHERE version_id = ?1", params![id, now])?;
            items.push(ClearItem { label, ok: true, message: "内容已清理，时间地图中保留占位".into() });
        }
        for id in &req.backups {
            let Some(b) = report.backups.iter().find(|b| &b.backup_id == id) else {
                items.push(ClearItem { label: "安全备份".into(), ok: false, message: "找不到这个安全备份".into() });
                continue;
            };
            let label = format!("安全备份“{}”", b.reason);
            if let Some(why) = &b.protected {
                items.push(ClearItem { label, ok: false, message: why.clone() });
                continue;
            }
            touched.extend(self.hashes_of(&tx, "backup_files", "backup_id", id)?);
            tx.execute("DELETE FROM backup_files WHERE backup_id = ?1", [id])?;
            tx.execute("UPDATE safety_backups SET status = 'cleared', cleared_at = ?2 WHERE backup_id = ?1", params![id, now])?;
            items.push(ClearItem { label, ok: true, message: "已清理，操作记录保留".into() });
        }
        // 不再被任何版本或备份引用的内容：先在事务中标记为待回收
        let mut orphans = Vec::new();
        for h in &touched {
            let used: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM version_files f JOIN versions v ON v.version_id = f.version_id
                                 WHERE v.project_id = ?1 AND f.content_hash = ?2)
                     OR EXISTS (SELECT 1 FROM backup_files f JOIN safety_backups b ON b.backup_id = f.backup_id
                                 WHERE b.project_id = ?1 AND f.content_hash = ?2)",
                params![project_id, h],
                |r| r.get(0),
            )?;
            if used {
                tx.execute(
                    "UPDATE content_objects SET ref_count = (
                        SELECT count(*) FROM version_files WHERE content_hash = ?2) + (SELECT count(*) FROM backup_files WHERE content_hash = ?2)
                      WHERE project_id = ?1 AND content_hash = ?2",
                    params![project_id, h],
                )?;
            } else {
                tx.execute(
                    "UPDATE content_objects SET state = 'pending_gc', ref_count = 0 WHERE project_id = ?1 AND content_hash = ?2",
                    params![project_id, h],
                )?;
                orphans.push(h.clone());
            }
        }
        tx.commit()?;
        drop(db);

        // 回收：删除成功的移除记录，失败的留作“待回收”
        let (freed, pending) = self.collect(project_id, &orphans)?;
        let ok = items.iter().filter(|i| i.ok).count();
        let status = if ok == items.len() { OpStatus::Succeeded } else { OpStatus::Failed };
        finish_op(&self.db(), &op_id, status, &format!("清理 {ok}/{} 项，释放 {} 字节", items.len(), freed))?;
        Ok(ClearResult { items, freed_bytes: freed, pending_gc_bytes: pending })
    }

    fn hashes_of(&self, tx: &rusqlite::Transaction<'_>, table: &str, col: &str, id: &str) -> CoreResult<Vec<String>> {
        let mut stmt = tx.prepare(&format!("SELECT DISTINCT content_hash FROM {table} WHERE {col} = ?1 AND content_hash IS NOT NULL"))?;
        let v = stmt.query_map([id], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
        Ok(v)
    }

    /// 删除内容对象文件及其缩略图；返回（实际释放字节数, 仍待回收字节数）。
    fn collect(&self, project_id: &str, hashes: &[String]) -> CoreResult<(i64, i64)> {
        let store = ObjectStore::new(&self.project_store_dir(project_id));
        let (mut freed, mut pending) = (0i64, 0i64);
        for h in hashes {
            let size: i64 = self
                .db()
                .query_row(
                    "SELECT byte_size FROM content_objects WHERE project_id = ?1 AND content_hash = ?2",
                    params![project_id, h],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            match crate::ops::failpoint::hit("gc").and_then(|_| store.remove(h)) {
                Ok(n) => {
                    freed += n as i64;
                    let thumb = self.project_store_dir(project_id).join("thumbs").join(&h[..2]).join(format!("{h}.png"));
                    if let Ok(m) = std::fs::metadata(&thumb) {
                        if std::fs::remove_file(&thumb).is_ok() {
                            freed += m.len() as i64;
                        }
                    }
                    self.db().execute(
                        "DELETE FROM content_objects WHERE project_id = ?1 AND content_hash = ?2 AND state = 'pending_gc'",
                        params![project_id, h],
                    )?;
                }
                Err(_) => pending += size,
            }
        }
        Ok((freed, pending))
    }

    fn retry_pending_gc(&self, project_id: &str) -> CoreResult<()> {
        let hashes: Vec<String> = {
            let db = self.db();
            let mut stmt = db.prepare("SELECT content_hash FROM content_objects WHERE project_id = ?1 AND state = 'pending_gc'")?;
            stmt.query_map([project_id], |r| r.get(0))?.collect::<Result<_, _>>()?
        };
        if !hashes.is_empty() {
            self.collect(project_id, &hashes)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::SourceRef;
    use crate::progress::NoProgress;
    use crate::version::SaveRequest;

    struct Env {
        _tmp: tempfile::TempDir,
        core: Core,
        work: std::path::PathBuf,
        pid: String,
    }

    fn env() -> Env {
        crate::ops::failpoint::clear();
        let tmp = tempfile::tempdir().unwrap();
        let core = Core::open(tmp.path().join("data")).unwrap();
        let work = tmp.path().join("w");
        std::fs::create_dir_all(&work).unwrap();
        let pid = core.add_project(&work, None).unwrap().project_id;
        Env { _tmp: tmp, core, work, pid }
    }

    fn save(e: &Env) -> String {
        e.core
            .save_version(&SaveRequest { project_id: e.pid.clone(), request_id: crate::new_id(), name: String::new(), note: String::new() }, &NoProgress)
            .unwrap()
            .version_id
    }

    fn clear(e: &Env, versions: &[&str], backups: &[&str], schemes: &[&str]) -> ClearResult {
        e.core
            .clear_storage(
                &e.pid,
                &ClearRequest {
                    request_id: crate::new_id(),
                    versions: versions.iter().map(|s| s.to_string()).collect(),
                    backups: backups.iter().map(|s| s.to_string()).collect(),
                    schemes: schemes.iter().map(|s| s.to_string()).collect(),
                },
            )
            .unwrap()
    }

    /// AC-0031、AT-14：线性历史 V1→V2→V3 中 V2 可清理并显示占位，V1、V3 仍可用；共享内容不被删除。
    #[test]
    fn clear_middle_version() {
        let e = env();
        std::fs::write(e.work.join("shared.txt"), "same in all").unwrap();
        std::fs::write(e.work.join("a.txt"), "1").unwrap();
        let v1 = save(&e);
        std::fs::write(e.work.join("a.txt"), "only in v2 ".repeat(100)).unwrap();
        let v2 = save(&e);
        std::fs::write(e.work.join("a.txt"), "3").unwrap();
        let v3 = save(&e);

        let report = e.core.storage_report(&e.pid).unwrap();
        let r2 = report.versions.iter().find(|v| v.version.version_id == v2).unwrap();
        assert!(r2.protected.is_none());
        assert_eq!(r2.exclusive_bytes, 1100);
        assert!(report.versions.iter().find(|v| v.version.version_id == v3).unwrap().protected.is_some(), "默认历史末端受保护");

        let r = clear(&e, &[&v2], &[], &[]);
        assert!(r.items[0].ok);
        assert_eq!(r.freed_bytes, 1100);
        let map = e.core.time_map(&e.pid).unwrap();
        assert!(map.nodes.iter().find(|n| n.version_id == v2).unwrap().cleared);
        assert!(e.core.version_files(&e.pid, &v2).is_err());
        for v in [&v1, &v3] {
            let files = e.core.version_files(&e.pid, v).unwrap();
            assert!(files.iter().all(|f| f.available), "共享内容不被删除");
        }
        let trail = e.core.file_trail(&e.pid, "a.txt", &crate::scheme::SwitchTarget::Default).unwrap();
        assert!(trail.incomplete, "轨迹提示记录不完整");
    }

    /// AC-0031：清理方案后其起点和末端可清理，名称可重新使用；活动方案不能清理。
    #[test]
    fn clear_scheme_releases_versions() {
        let e = env();
        std::fs::write(e.work.join("a.txt"), "1").unwrap();
        let v1 = save(&e);
        std::fs::write(e.work.join("a.txt"), "2").unwrap();
        save(&e);
        let s = e.core.create_scheme(&e.pid, &v1, "试验").unwrap();
        let rep = e.core.storage_report(&e.pid).unwrap();
        assert!(rep.versions.iter().find(|v| v.version.version_id == v1).unwrap().protected.as_deref().unwrap().contains("试验"));

        let r = clear(&e, &[&v1], &[], &[&s.scheme_id]);
        assert!(r.items.iter().all(|i| i.ok), "{:?}", r.items);
        assert!(e.core.list_schemes(&e.pid).unwrap().schemes.is_empty());
        let head = crate::project::load_project(&e.core.db(), &e.pid).unwrap().default_head.unwrap();
        e.core.create_scheme(&e.pid, &head, "试验").unwrap();
    }

    #[test]
    fn active_scheme_and_incomplete_backup_are_protected() {
        let e = env();
        std::fs::write(e.work.join("a.txt"), "1").unwrap();
        std::fs::write(e.work.join("b.txt"), "1").unwrap();
        let v1 = save(&e);
        let s = e.core.create_scheme(&e.pid, &v1, "当前").unwrap();
        let check = e.core.check_switch(&e.pid, &crate::scheme::SwitchTarget::Scheme { scheme_id: s.scheme_id.clone() }, &NoProgress).unwrap();
        e.core
            .switch_scheme(
                &crate::scheme::SwitchRequest {
                    project_id: e.pid.clone(),
                    request_id: crate::new_id(),
                    target: crate::scheme::SwitchTarget::Scheme { scheme_id: s.scheme_id.clone() },
                    fingerprint: check.impact.unwrap().fingerprint,
                },
                &NoProgress,
            )
            .unwrap();
        let r = clear(&e, &[], &[], &[&s.scheme_id]);
        assert!(!r.items[0].ok);

        // 未完成操作关联的备份不可清理
        std::fs::write(e.work.join("a.txt"), "2").unwrap();
        std::fs::write(e.work.join("b.txt"), "2").unwrap();
        save(&e);
        crate::ops::failpoint::arm("execute_item", 1, ErrorCode::Io);
        let plan = e.core.plan_restore(&e.pid, &v1, &NoProgress).unwrap();
        let op = e.core
            .restore_version(
                &crate::ops::restore::WorkspaceOpRequest { project_id: e.pid.clone(), request_id: crate::new_id(), version_id: v1, fingerprint: plan.fingerprint },
                &NoProgress,
            )
            .unwrap();
        let backup = op.backup_id.unwrap();
        let r = clear(&e, &[], &[&backup], &[]);
        assert!(!r.items[0].ok);
        e.core.resolve_incomplete(&e.pid, &op.operation_id).unwrap();
        let r = clear(&e, &[], &[&backup], &[]);
        assert!(r.items[0].ok);
        assert!(e.core.source_files(&e.pid, &SourceRef::Backup { backup_id: backup }).is_err());
    }

    /// LFVM-AT-06：清理后回收失败记为待回收，不影响其他版本；下次打开存储管理时重试。
    #[test]
    fn failed_gc_is_retried() {
        let e = env();
        std::fs::write(e.work.join("a.txt"), "1").unwrap();
        let v1 = save(&e);
        std::fs::write(e.work.join("a.txt"), "2").unwrap();
        save(&e);
        crate::ops::failpoint::arm("gc", 0, ErrorCode::FileInUse);
        let r = clear(&e, &[&v1], &[], &[]);
        assert_eq!((r.freed_bytes, r.pending_gc_bytes), (0, 1));
        let rep = e.core.storage_report(&e.pid).unwrap();
        assert_eq!(rep.usage.pending_gc_bytes, 0, "重试后已回收");
    }
}
