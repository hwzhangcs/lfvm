//! 查看当前变化并保存版本（SRS 3.3.1.3，规则 R-01，LFVM-C-02）。
//!
//! 保存流程：
//! 1. 扫描工作区，与比较基准对比；无变化时不重复保存；
//! 2. 把内容尚未存储的文件复制进内容对象库，复制时再次计算摘要，与扫描结果不一致即中止；
//! 3. 重新扫描核对：期间有文件被新增、删除或改写则中止（LFVM-AT-01）；
//! 4. 在一个数据库事务内登记版本、清单、规则快照，并更新所属路线的末端；
//! 5. 任一步失败：不产生版本，末端不变，回收本次新写入但未登记的内容（LFVM-AT-06）。

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::changes::{self, ChangeSet, Manifest};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::exclude::{self, Matcher};
use crate::paths::path_key;
use crate::progress::{Progress, Stage};
use crate::project::{ProjectRow, baseline_id, load_project};
use crate::scan::{self, CacheEntry, Scan, ScanCache};
use crate::store::ObjectStore;
use crate::{Core, new_id, now_ms};

pub const MAX_NAME_CHARS: usize = 100;
pub const MAX_NOTE_CHARS: usize = 500;

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SaveRequest {
    pub project_id: String,
    /// 界面为每次“确认保存”生成的标识；重复提交同一请求不会重复保存（LFVM-Q-05）。
    pub request_id: String,
    pub name: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SaveResult {
    pub version_id: String,
    pub seq: u32,
    pub added: u32,
    pub modified: u32,
    pub deleted: u32,
    pub file_count: u32,
    pub directory_count: u32,
}

/// 扫描的中间结果，供“查看变化”和“保存”共用。
struct Prepared {
    project: ProjectRow,
    matcher: Matcher,
    rules_json: String,
    baseline: Option<String>,
    scan: Scan,
    changes: ChangeSet,
}

impl Core {
    /// 扫描工作区并返回相对比较基准的变化。
    pub fn current_changes(&self, project_id: &str, progress: &dyn Progress) -> CoreResult<ChangeSet> {
        Ok(self.prepare(project_id, progress)?.changes)
    }

    fn prepare(&self, project_id: &str, progress: &dyn Progress) -> CoreResult<Prepared> {
        let (project, rules, cache, baseline, baseline_manifest, baseline_rules) = {
            let db = self.db();
            let project = load_project(&db, project_id)?;
            let rules = exclude::load_rules(&db, project_id)?;
            let cache = scan::load_cache(&db, project_id)?;
            let baseline = baseline_id(&db, &project)?;
            let (manifest, snapshot) = match &baseline {
                Some(v) => (changes::load_manifest(&db, v)?, Some(rules_snapshot_of(&db, v)?)),
                None => (Manifest::new(), None),
            };
            (project, rules, cache, baseline, manifest, snapshot)
        };
        if !project.root.is_dir() {
            return Err(CoreError::new(ErrorCode::NotFound, "项目文件夹无法访问").with_path(&project.root));
        }
        let matcher = Matcher::new(&rules, project.policy)?;
        let rules_json = exclude::snapshot_json(&rules, project.policy);
        let scan = scan::scan(&project.root, &matcher, project.policy, &cache, progress)?;
        scan::store_cache(&mut self.db(), project_id, &scan, project.policy)?;

        let rules_changed = baseline_rules.is_some_and(|old| {
            exclude::snapshot_json(&exclude::parse_snapshot(&old), project.policy) != rules_json
        });
        let changes = changes::diff(baseline.as_deref(), &baseline_manifest, &scan, &matcher, rules_changed);
        Ok(Prepared { project, matcher, rules_json, baseline, scan, changes })
    }

    /// 保存当前文件夹状态为新版本。
    pub fn save_version(&self, req: &SaveRequest, progress: &dyn Progress) -> CoreResult<SaveResult> {
        let name = req.name.trim().to_owned();
        let note = req.note.trim().to_owned();
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(CoreError::new(ErrorCode::InvalidInput, format!("版本名称不能超过 {MAX_NAME_CHARS} 个字")));
        }
        if note.chars().count() > MAX_NOTE_CHARS {
            return Err(CoreError::new(ErrorCode::InvalidInput, format!("版本说明不能超过 {MAX_NOTE_CHARS} 个字")));
        }
        let _guard = self.begin_write(&req.project_id)?;
        {
            let db = self.db();
            let project = load_project(&db, &req.project_id)?;
            if project.incomplete {
                return Err(CoreError::new(
                    ErrorCode::IncompleteOperation,
                    "有未完成的操作需要先处理，处理后才能保存新版本",
                ));
            }
            if let Some(done) = previous_request(&db, &req.project_id, &req.request_id)? {
                return done;
            }
            db.execute(
                "INSERT INTO operations (operation_id, project_id, request_id, type, target_ref, created_at)
                 VALUES (?1, ?2, ?3, 'save', ?4, ?5)",
                params![new_id(), req.project_id, req.request_id, project.active_scheme_id.as_deref().unwrap_or("default"), now_ms()],
            )?;
        }

        let store = ObjectStore::new(&self.project_store_dir(&req.project_id));
        let mut created: Vec<String> = Vec::new();
        let result = self.save_inner(req, &name, &note, &store, &mut created, progress);

        let (status, message, version) = match &result {
            Ok(r) => ("succeeded", String::new(), Some(r.version_id.clone())),
            Err(e) if e.code == ErrorCode::Cancelled => ("cancelled", e.message.clone(), None),
            Err(e) => ("failed", e.message.clone(), None),
        };
        if result.is_err() {
            self.discard_unregistered(&req.project_id, &store, &created);
        }
        let message: String = message.chars().take(500).collect();
        self.db().execute(
            "UPDATE operations SET status = ?1, result_message = ?2, resolved_version_id = ?3, finished_at = ?4,
                    phase = 'committing'
              WHERE project_id = ?5 AND request_id = ?6",
            params![status, message, version, now_ms(), req.project_id, req.request_id],
        )?;
        result
    }

    fn save_inner(
        &self,
        req: &SaveRequest,
        name: &str,
        note: &str,
        store: &ObjectStore,
        created: &mut Vec<String>,
        progress: &dyn Progress,
    ) -> CoreResult<SaveResult> {
        let prep = self.prepare(&req.project_id, progress)?;
        if let Some(p) = prep.changes.problems.first() {
            return Err(CoreError::new(
                p.code,
                format!("有 {} 个文件或文件夹需要先处理：{}", prep.changes.problems.len(), p.message),
            )
            .with_path(&p.path));
        }
        if !prep.changes.has_changes {
            return Err(CoreError::new(ErrorCode::NothingToSave, "没有需要保存的变化"));
        }

        // 2. 写入内容对象
        store.ensure_dirs()?;
        let files: Vec<_> = prep.scan.entries.values().filter(|e| !e.entry_type.is_dir()).collect();
        let pending: Vec<_> = files
            .iter()
            .filter(|e| e.hash.as_deref().is_none_or(|h| !store.contains(h)))
            .collect();
        let total: u64 = pending.iter().map(|e| e.size).sum();
        let mut done = 0u64;
        progress.report(Stage::Storing, 0, total);
        for e in pending {
            progress.check()?;
            let src = e.rel.to_path(&prep.project.root);
            let got = store.ingest(&src, e.hash.as_deref())?;
            if got.created {
                created.push(got.hash);
            }
            done += e.size;
            progress.report(Stage::Storing, done, total);
        }

        // 3. 再次核对工作区
        progress.report(Stage::Verifying, 0, 0);
        let cache: ScanCache = prep
            .scan
            .entries
            .iter()
            .filter_map(|(k, e)| {
                e.hash.as_ref().map(|h| {
                    (k.clone(), CacheEntry { size: e.size, mtime_ns: e.mtime_ns, file_id: e.file_id.clone(), hash: h.clone() })
                })
            })
            .collect();
        let again = scan::scan(&prep.project.root, &prep.matcher, prep.project.policy, &cache, progress)?;
        let same = again.entries.len() == prep.scan.entries.len()
            && again.entries.iter().all(|(k, e)| {
                prep.scan.entries.get(k).is_some_and(|o| o.entry_type == e.entry_type && o.hash == e.hash)
            });
        if !same {
            return Err(CoreError::new(ErrorCode::ChangedExternally, "文件在保存过程中发生变化，请重新查看后保存"));
        }
        progress.check()?;

        // 4. 一次性登记
        progress.report(Stage::Committing, 0, 0);
        let c = &prep.changes.counts;
        let mut db = self.db();
        let tx = db.transaction()?;
        let project = load_project(&tx, &req.project_id)?;
        // 扫描期间路线末端不可能变化（同一项目的写操作互斥），这里再确认一次。
        if baseline_id(&tx, &project)? != prep.baseline {
            return Err(CoreError::new(ErrorCode::Busy, "项目状态已变化，请重新查看后保存"));
        }
        let seq: u32 = tx.query_row("SELECT next_seq FROM projects WHERE project_id = ?1", [&req.project_id], |r| r.get(0))?;
        let version_id = new_id();
        let excluded: Vec<&str> = prep.scan.excluded.iter().map(|r| r.as_str()).collect();
        tx.execute(
            "INSERT INTO versions (version_id, project_id, seq, parent_version_id, origin_scheme_id, name, note, created_at,
                                   file_count, directory_count, added_count, modified_count, deleted_count,
                                   exclusion_rules_snapshot, excluded_paths_observed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                version_id,
                req.project_id,
                seq,
                prep.baseline,
                project.active_scheme_id,
                name,
                note,
                now_ms(),
                prep.changes.file_count,
                prep.changes.directory_count,
                c.added,
                c.modified,
                c.deleted,
                prep.rules_json,
                serde_json::to_string(&excluded).unwrap_or_else(|_| "[]".into()),
            ],
        )?;
        insert_manifest(&tx, &version_id, &prep.scan)?;
        upsert_objects(&tx, &req.project_id, &prep.scan)?;
        match &project.active_scheme_id {
            Some(sid) => {
                tx.execute("UPDATE schemes SET head_version_id = ?1 WHERE scheme_id = ?2", params![version_id, sid])?;
            }
            None => {
                tx.execute(
                    "UPDATE projects SET default_head_version_id = ?1 WHERE project_id = ?2",
                    params![version_id, req.project_id],
                )?;
            }
        }
        tx.execute("UPDATE projects SET next_seq = next_seq + 1 WHERE project_id = ?1", [&req.project_id])?;
        tx.commit()?;

        Ok(SaveResult {
            version_id,
            seq,
            added: c.added,
            modified: c.modified,
            deleted: c.deleted,
            file_count: prep.changes.file_count,
            directory_count: prep.changes.directory_count,
        })
    }

    /// 回收本次新写入、但没有登记到数据库的内容对象。
    fn discard_unregistered(&self, project_id: &str, store: &ObjectStore, created: &[String]) {
        let db = self.db();
        for h in created {
            let registered: bool = db
                .query_row(
                    "SELECT 1 FROM content_objects WHERE project_id = ?1 AND content_hash = ?2",
                    params![project_id, h],
                    |_| Ok(()),
                )
                .optional()
                .is_ok_and(|r| r.is_some());
            if !registered {
                let _ = store.remove(h);
            }
        }
    }

    /// 上次程序异常退出时进行中的保存：工作区未被改动，记为失败，并回收未登记的内容。
    pub(crate) fn recover_interrupted_saves(&self, project_id: &str) -> CoreResult<()> {
        let n = self.db().execute(
            "UPDATE operations SET status = 'failed', result_message = '程序在保存过程中意外退出，未产生版本',
                    finished_at = ?2
              WHERE project_id = ?1 AND type = 'save' AND status = 'running'",
            params![project_id, now_ms()],
        )?;
        if n > 0 {
            self.collect_orphan_objects(project_id)?;
        }
        Ok(())
    }

    /// 删除对象目录中没有登记的内容对象和遗留的临时文件。
    fn collect_orphan_objects(&self, project_id: &str) -> CoreResult<()> {
        let dir = self.project_store_dir(project_id);
        let store = ObjectStore::new(&dir);
        if let Ok(rd) = std::fs::read_dir(dir.join("tmp")) {
            for f in rd.flatten() {
                let _ = std::fs::remove_file(f.path());
            }
        }
        let registered: std::collections::HashSet<String> = {
            let db = self.db();
            let mut stmt = db.prepare("SELECT content_hash FROM content_objects WHERE project_id = ?1")?;
            stmt.query_map([project_id], |r| r.get(0))?.collect::<Result<_, _>>()?
        };
        let Ok(shards) = std::fs::read_dir(dir.join("objects")) else { return Ok(()) };
        for shard in shards.flatten() {
            let Ok(objs) = std::fs::read_dir(shard.path()) else { continue };
            for obj in objs.flatten() {
                let name = obj.file_name().to_string_lossy().into_owned();
                if !registered.contains(&name) {
                    let _ = store.remove(&name);
                }
            }
        }
        Ok(())
    }
}

fn rules_snapshot_of(conn: &Connection, version_id: &str) -> CoreResult<String> {
    Ok(conn.query_row("SELECT exclusion_rules_snapshot FROM versions WHERE version_id = ?1", [version_id], |r| r.get(0))?)
}

/// 同一请求已处理过时返回其结果。
fn previous_request(conn: &Connection, project_id: &str, request_id: &str) -> CoreResult<Option<CoreResult<SaveResult>>> {
    let row: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT status, resolved_version_id FROM operations WHERE project_id = ?1 AND request_id = ?2",
            params![project_id, request_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(row.map(|(status, version)| match (status.as_str(), version) {
        ("succeeded", Some(v)) => conn
            .query_row(
                "SELECT seq, added_count, modified_count, deleted_count, file_count, directory_count
                   FROM versions WHERE version_id = ?1",
                [&v],
                |r| {
                    Ok(SaveResult {
                        version_id: v.clone(),
                        seq: r.get(0)?,
                        added: r.get(1)?,
                        modified: r.get(2)?,
                        deleted: r.get(3)?,
                        file_count: r.get(4)?,
                        directory_count: r.get(5)?,
                    })
                },
            )
            .map_err(CoreError::from),
        ("running", _) => Err(CoreError::new(ErrorCode::Busy, "正在保存，请稍候")),
        _ => Err(CoreError::new(ErrorCode::InvalidInput, "这次保存请求已经结束，请重新点击保存")),
    }))
}

fn insert_manifest(conn: &Connection, version_id: &str, scan: &Scan) -> CoreResult<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO version_files (version_id, path_key, relative_path, name_key, ext_key, entry_type, size, content_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    let lower = lfvm_platform::CasePolicy::Insensitive;
    for (key, e) in &scan.entries {
        stmt.execute(params![
            version_id,
            key,
            e.rel.as_str(),
            path_key(e.rel.file_name(), lower),
            e.rel.extension().map(|x| path_key(x, lower)).unwrap_or_default(),
            e.entry_type.as_str(),
            (!e.entry_type.is_dir()).then_some(e.size as i64),
            e.hash,
        ])?;
    }
    Ok(())
}

fn upsert_objects(conn: &Connection, project_id: &str, scan: &Scan) -> CoreResult<()> {
    let mut refs: HashMap<&str, (u64, u32)> = HashMap::new();
    for e in scan.entries.values() {
        if let Some(h) = &e.hash {
            let slot = refs.entry(h).or_insert((e.size, 0));
            slot.1 += 1;
        }
    }
    let mut stmt = conn.prepare(
        "INSERT INTO content_objects (project_id, content_hash, byte_size, state, ref_count)
         VALUES (?1, ?2, ?3, 'ready', ?4)
         ON CONFLICT (project_id, content_hash) DO UPDATE SET ref_count = ref_count + excluded.ref_count, state = 'ready'",
    )?;
    for (h, (size, n)) in refs {
        stmt.execute(params![project_id, h, size as i64, n])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::NoProgress;
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

    fn save(e: &Env, name: &str) -> CoreResult<SaveResult> {
        e.core.save_version(
            &SaveRequest { project_id: e.pid.clone(), request_id: new_id(), name: name.into(), note: String::new() },
            &NoProgress,
        )
    }

    #[test]
    fn empty_folder_can_be_first_version_then_nothing_to_save() {
        let e = env();
        let r = save(&e, "").unwrap();
        assert_eq!((r.seq, r.file_count), (1, 0));
        assert_eq!(save(&e, "").unwrap_err().code, ErrorCode::NothingToSave);
    }

    #[test]
    fn save_updates_default_head_and_parent_chain() {
        let e = env();
        std::fs::write(e.work.join("a.txt"), b"1").unwrap();
        std::fs::create_dir(e.work.join("empty")).unwrap();
        let v1 = save(&e, "第一版").unwrap();
        assert_eq!((v1.added, v1.file_count, v1.directory_count), (1, 1, 1));

        std::fs::write(e.work.join("a.txt"), b"2").unwrap();
        std::fs::write(e.work.join("b.txt"), b"2").unwrap();
        let cs = e.core.current_changes(&e.pid, &NoProgress).unwrap();
        assert_eq!(cs.baseline_version_id.as_deref(), Some(v1.version_id.as_str()));
        assert_eq!((cs.counts.added, cs.counts.modified), (1, 1));

        let v2 = save(&e, "").unwrap();
        assert_eq!((v2.seq, v2.added, v2.modified), (2, 1, 1));
        let db = e.core.db();
        let parent: Option<String> = db
            .query_row("SELECT parent_version_id FROM versions WHERE version_id = ?1", [&v2.version_id], |r| r.get(0))
            .unwrap();
        assert_eq!(parent.as_deref(), Some(v1.version_id.as_str()));
        let head: String = db
            .query_row("SELECT default_head_version_id FROM projects WHERE project_id = ?1", [&e.pid], |r| r.get(0))
            .unwrap();
        assert_eq!(head, v2.version_id);
        // 相同内容只存一份：“2” 被两个文件引用
        let refs: i64 = db
            .query_row(
                "SELECT ref_count FROM content_objects WHERE content_hash = ?1",
                [crate::hash::hash_bytes(b"2")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(refs, 2);
    }

    #[test]
    fn duplicate_request_does_not_save_twice() {
        let e = env();
        std::fs::write(e.work.join("a.txt"), b"1").unwrap();
        let req = SaveRequest { project_id: e.pid.clone(), request_id: "r1".into(), name: String::new(), note: String::new() };
        let a = e.core.save_version(&req, &NoProgress).unwrap();
        let b = e.core.save_version(&req, &NoProgress).unwrap();
        assert_eq!(a.version_id, b.version_id);
        let n: i64 = e.core.db().query_row("SELECT count(*) FROM versions", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn rule_change_alone_is_a_change_and_excluded_files_not_deleted() {
        let e = env();
        std::fs::create_dir(e.work.join("cache")).unwrap();
        std::fs::write(e.work.join("cache/x"), b"x").unwrap();
        save(&e, "").unwrap();
        let mut rules: Vec<_> = e.core.exclusion_rules(&e.pid).unwrap().into_iter().map(|v| v.rule).collect();
        rules.push(exclude::ExclusionRule {
            relative_path: "cache".into(),
            entry_type: exclude::RuleType::Directory,
            is_system_default: false,
            enabled: true,
        });
        e.core.save_exclusion_rules(&e.pid, &rules).unwrap();
        let cs = e.core.current_changes(&e.pid, &NoProgress).unwrap();
        assert!(cs.rules_changed && cs.has_changes);
        assert_eq!((cs.counts.deleted, cs.counts.excluded_now), (0, 2));
        let v2 = save(&e, "").unwrap();
        assert_eq!(v2.deleted, 0);
        assert!(e.work.join("cache/x").is_file(), "排除不得删除磁盘上的文件");
    }

    #[test]
    fn failed_save_leaves_no_version_and_no_orphans() {
        let e = env();
        std::fs::write(e.work.join("a.txt"), b"1").unwrap();
        // 工作区含未排除的链接时保存失败
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(e.work.join("a.txt"), e.work.join("link")).unwrap();
            let err = e
                .core
                .save_version(
                    &SaveRequest { project_id: e.pid.clone(), request_id: "x".into(), name: String::new(), note: String::new() },
                    &NoProgress,
                )
                .unwrap_err();
            assert_eq!(err.code, ErrorCode::LinkNotFollowed);
            let n: i64 = e.core.db().query_row("SELECT count(*) FROM versions", [], |r| r.get(0)).unwrap();
            assert_eq!(n, 0);
            let status: String = e
                .core
                .db()
                .query_row("SELECT status FROM operations WHERE request_id = 'x'", [], |r| r.get(0))
                .unwrap();
            assert_eq!(status, "failed");
        }
    }

    #[test]
    fn interrupted_save_is_recovered_on_open() {
        let e = env();
        let store = ObjectStore::new(&e.core.project_store_dir(&e.pid));
        let orphan = store.ingest_bytes(b"orphan").unwrap();
        e.core
            .db()
            .execute(
                "INSERT INTO operations (operation_id, project_id, request_id, type, target_ref, created_at)
                 VALUES ('op', ?1, 'r', 'save', 'default', 0)",
                [&e.pid],
            )
            .unwrap();
        e.core.open_project(&e.pid).unwrap();
        assert!(!store.contains(&orphan.hash));
        let status: String =
            e.core.db().query_row("SELECT status FROM operations WHERE operation_id = 'op'", [], |r| r.get(0)).unwrap();
        assert_eq!(status, "failed");
    }

    /// LFVM-AT-01：保存过程中以相同大小改写文件、新增或删除文件，保存中止，不产生版本，末端不变。
    #[test]
    fn external_change_during_save_aborts() {
        use std::sync::atomic::{AtomicBool, Ordering};

        struct Meddler<F: Fn() + Sync> {
            fired: AtomicBool,
            act: F,
        }
        impl<F: Fn() + Sync> Progress for Meddler<F> {
            fn report(&self, stage: Stage, _: u64, _: u64) {
                if stage == Stage::Storing && !self.fired.swap(true, Ordering::SeqCst) {
                    (self.act)();
                }
            }
            fn is_cancelled(&self) -> bool {
                false
            }
        }

        let e = env();
        std::fs::write(e.work.join("a.txt"), b"1111").unwrap();
        let v1 = save(&e, "").unwrap();

        let a = e.work.join("a.txt");
        let b = e.work.join("b.txt");
        let actions: Vec<Box<dyn Fn() + Sync>> = vec![
            // 相同大小改写（修改时间也会变化）
            Box::new(move || std::fs::write(&a, b"2222").unwrap()),
            Box::new(move || std::fs::write(&b, b"new").unwrap()),
        ];
        for (i, act) in actions.into_iter().enumerate() {
            std::fs::write(e.work.join(format!("trigger{i}.txt")), b"x").unwrap();
            let m = Meddler { fired: AtomicBool::new(false), act };
            let err = e
                .core
                .save_version(
                    &SaveRequest { project_id: e.pid.clone(), request_id: new_id(), name: String::new(), note: String::new() },
                    &m,
                )
                .unwrap_err();
            assert_eq!(err.code, ErrorCode::ChangedExternally, "第 {i} 种外部修改");
            let head: String = e
                .core
                .db()
                .query_row("SELECT default_head_version_id FROM projects WHERE project_id = ?1", [&e.pid], |r| r.get(0))
                .unwrap();
            assert_eq!(head, v1.version_id, "末端不变");
        }
        let n: i64 = e.core.db().query_row("SELECT count(*) FROM versions", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1, "不产生版本");
        // 刚才失败的保存不留下未登记的内容对象
        let objects = std::fs::read_dir(e.core.project_store_dir(&e.pid).join("objects"))
            .unwrap()
            .flat_map(|d| std::fs::read_dir(d.unwrap().path()).unwrap())
            .count();
        let registered: i64 = e.core.db().query_row("SELECT count(*) FROM content_objects", [], |r| r.get(0)).unwrap();
        assert_eq!(objects as i64, registered);
    }

    #[test]
    fn rejects_overlong_name() {
        let e = env();
        let err = save(&e, &"名".repeat(101)).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
    }
}
