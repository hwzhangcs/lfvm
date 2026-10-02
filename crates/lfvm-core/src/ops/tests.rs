//! 操作引擎测试，对应 SRS 附件 C 的验收条件与异常测试。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::failpoint;
use super::restore::WorkspaceOpRequest;
use super::single::{FileRestoreRequest, FileTarget};
use super::*;
use crate::content::SourceRef;
use crate::exclude::{ExclusionRule, RuleType};
use crate::progress::{NoProgress, Progress, Stage};
use crate::version::SaveRequest;

struct Env {
    _tmp: tempfile::TempDir,
    core: Core,
    work: PathBuf,
    pid: String,
    outside: PathBuf,
}

fn env() -> Env {
    failpoint::clear();
    let tmp = tempfile::tempdir().unwrap();
    let core = Core::open(tmp.path().join("data")).unwrap();
    let work = tmp.path().join("w");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let pid = core.add_project(&work, None).unwrap().project_id;
    Env { _tmp: tmp, core, work, pid, outside }
}

impl Env {
    fn write(&self, p: &str, s: &str) {
        let f = self.work.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, s).unwrap();
    }
    fn read(&self, p: &str) -> Option<String> {
        std::fs::read_to_string(self.work.join(p)).ok()
    }
    fn save(&self) -> String {
        self.core
            .save_version(
                &SaveRequest {
                    project_id: self.pid.clone(),
                    request_id: new_id(),
                    name: String::new(),
                    note: String::new(),
                },
                &NoProgress,
            )
            .unwrap()
            .version_id
    }
    fn restore(&self, version: &str, progress: &dyn Progress) -> OperationResult {
        let plan = self.core.plan_restore(&self.pid, version, &NoProgress).unwrap();
        self.core
            .restore_version(
                &WorkspaceOpRequest {
                    project_id: self.pid.clone(),
                    request_id: new_id(),
                    version_id: version.into(),
                    fingerprint: plan.fingerprint,
                },
                progress,
            )
            .unwrap()
    }
    fn head(&self) -> Option<String> {
        self.core
            .db()
            .query_row("SELECT default_head_version_id FROM projects WHERE project_id = ?1", [&self.pid], |r| r.get(0))
            .unwrap()
    }
    fn exclude_dir(&self, p: &str) {
        let mut rules: Vec<_> = self.core.exclusion_rules(&self.pid).unwrap().into_iter().map(|v| v.rule).collect();
        rules.push(ExclusionRule {
            relative_path: p.into(),
            entry_type: RuleType::Directory,
            is_system_default: false,
            enabled: true,
        });
        self.core.save_exclusion_rules(&self.pid, &rules).unwrap();
    }
    fn incomplete(&self) -> bool {
        self.core.overview(&self.pid).unwrap().project.incomplete
    }
}

/// 在指定阶段第一次报告进度时执行一次动作，用于模拟“操作进行中”的外部修改。
struct At<F: Fn() + Sync> {
    stage: Stage,
    fired: AtomicBool,
    act: F,
    cancel_after: bool,
}

impl<F: Fn() + Sync> Progress for At<F> {
    fn report(&self, stage: Stage, _: u64, _: u64) {
        if stage == self.stage && !self.fired.swap(true, Ordering::SeqCst) {
            (self.act)();
        }
    }
    fn is_cancelled(&self) -> bool {
        self.cancel_after && self.fired.load(Ordering::SeqCst)
    }
}

fn at<F: Fn() + Sync>(stage: Stage, act: F) -> At<F> {
    At { stage, fired: AtomicBool::new(false), act, cancel_after: false }
}

// ───────────────────────── 整版恢复 ─────────────────────────

/// AC-0016：新增、替换、删除、类型变化后整版恢复，结果与目标版本一致；恢复后各末端不变。
#[test]
fn restore_whole_version_roundtrip() {
    let e = env();
    e.write("a.txt", "1");
    e.write("d/b.txt", "2");
    e.write("t/inner.txt", "dir");
    std::fs::create_dir_all(e.work.join("empty")).unwrap();
    let v1 = e.save();
    e.write("a.txt", "modified");
    e.write("c.txt", "new");
    std::fs::remove_file(e.work.join("d/b.txt")).unwrap();
    std::fs::remove_dir_all(e.work.join("t")).unwrap();
    e.write("t", "now a file");
    std::fs::remove_dir(e.work.join("empty")).unwrap();
    let v2 = e.save();
    assert_eq!(e.head().as_deref(), Some(v2.as_str()));

    let plan = e.core.plan_restore(&e.pid, &v1, &NoProgress).unwrap();
    assert!(plan.conflicts.is_empty());
    assert_eq!(plan.counts.type_change, 1);

    let r = e.restore(&v1, &NoProgress);
    assert_eq!(r.status, OpStatus::Succeeded, "{r:?}");
    assert_eq!(e.read("a.txt").as_deref(), Some("1"));
    assert_eq!(e.read("d/b.txt").as_deref(), Some("2"));
    assert_eq!(e.read("t/inner.txt").as_deref(), Some("dir"));
    assert!(e.work.join("empty").is_dir());
    assert!(!e.work.join("c.txt").exists());
    // 整版恢复不改变路线末端（R-01），因此当前变化显示相对 V2 的差异
    assert_eq!(e.head().as_deref(), Some(v2.as_str()));
    // 被覆盖和删除的文件都在安全备份中
    let files = e.core.source_files(&e.pid, &SourceRef::Backup { backup_id: r.backup_id.clone().unwrap() }).unwrap();
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"a.txt") && paths.contains(&"c.txt") && paths.contains(&"t"));
    // 恢复回 V2 也成功
    assert_eq!(e.restore(&v2, &NoProgress).status, OpStatus::Succeeded);
    assert_eq!(e.read("t").as_deref(), Some("now a file"));
}

/// AC-0017：只新增文件时记录“无需备份”。
#[test]
fn only_additions_need_no_backup() {
    let e = env();
    e.write("a.txt", "1");
    let v1 = e.save();
    std::fs::remove_file(e.work.join("a.txt")).unwrap();
    let r = e.restore(&v1, &NoProgress);
    assert_eq!(r.status, OpStatus::Succeeded);
    assert!(r.backup_id.is_none());
    assert!(r.message.contains("无需备份"));
}

/// AC-0016、R-05：排除文件保持不变。
#[test]
fn excluded_files_are_untouched() {
    let e = env();
    e.write("a.txt", "1");
    let v1 = e.save();
    e.exclude_dir("cache");
    e.write("cache/big.bin", "keep me");
    e.write("a.txt", "2");
    let r = e.restore(&v1, &NoProgress);
    assert_eq!(r.status, OpStatus::Succeeded);
    assert_eq!(e.read("cache/big.bin").as_deref(), Some("keep me"));
    assert_eq!(e.read("a.txt").as_deref(), Some("1"));
}

/// LFVM-AT-09：排除了 a/cache，目标版本中 a 为文件：写入前报结构冲突，工作区不变。
#[test]
fn structure_conflict_aborts_before_writing() {
    let e = env();
    e.write("a", "file");
    let v1 = e.save();
    std::fs::remove_file(e.work.join("a")).unwrap();
    e.write("a/cache/x", "excluded");
    e.write("a/y", "managed");
    e.exclude_dir("a/cache");
    e.save();
    let plan = e.core.plan_restore(&e.pid, &v1, &NoProgress).unwrap();
    assert_eq!(plan.conflicts.len(), 1);
    let r = e
        .core
        .restore_version(
            &WorkspaceOpRequest {
                project_id: e.pid.clone(),
                request_id: new_id(),
                version_id: v1,
                fingerprint: plan.fingerprint,
            },
            &NoProgress,
        )
        .unwrap();
    assert_eq!(r.status, OpStatus::Failed);
    assert_eq!(e.read("a/y").as_deref(), Some("managed"));
    assert!(!e.incomplete());
}

/// AC-0018：备份失败时工作区没有任何写入，失败的备份不能作为恢复来源。
#[test]
fn backup_failure_leaves_workspace_untouched() {
    let e = env();
    e.write("a.txt", "1");
    e.write("b.txt", "1");
    let v1 = e.save();
    e.write("a.txt", "2");
    e.write("b.txt", "2");
    failpoint::arm("backup_verify", 1, ErrorCode::ContentCorrupted);
    let r = e.restore(&v1, &NoProgress);
    assert_eq!(r.status, OpStatus::Failed);
    assert!(r.message.contains("安全备份没有创建成功"));
    assert_eq!(e.read("a.txt").as_deref(), Some("2"));
    assert_eq!(e.read("b.txt").as_deref(), Some("2"));
    let ops = e.core.list_operations(&e.pid).unwrap();
    let b = ops[0].backup.as_ref().unwrap();
    assert_eq!(b.status, BackupStatus::Failed);
    assert!(!b.recoverable);
    assert!(e.core.source_files(&e.pid, &SourceRef::Backup { backup_id: b.backup_id.clone() }).is_err());
}

/// LFVM-AT-04：空间不足在备份前发现，工作区不变。
#[test]
fn no_space_fails_before_writing() {
    let e = env();
    e.write("a.txt", "1");
    let v1 = e.save();
    e.write("a.txt", "2");
    failpoint::arm("space", 0, ErrorCode::NoSpace);
    let r = e.restore(&v1, &NoProgress);
    assert_eq!(r.status, OpStatus::Failed);
    assert_eq!(e.read("a.txt").as_deref(), Some("2"));
}

/// LFVM-AT-02：安全备份就绪后、写入前修改目标文件：该项不被覆盖，操作停止。
#[test]
fn modified_after_backup_is_not_overwritten() {
    let e = env();
    e.write("a.txt", "1");
    let v1 = e.save();
    e.write("a.txt", "2");
    let path = e.work.join("a.txt");
    let p = at(Stage::Writing, || std::fs::write(&path, "user edit").unwrap());
    let r = e.restore(&v1, &p);
    assert_eq!(r.status, OpStatus::Failed, "{r:?}");
    assert_eq!(e.read("a.txt").as_deref(), Some("user edit"));
    assert_eq!(r.failed.len(), 1);
}

/// LFVM-AT-03、AT-10：写入中途失败 → 未完成；处置前不能保存或整版恢复；重试后成功，原记录状态不变。
#[test]
fn incomplete_then_retry() {
    let e = env();
    e.write("a.txt", "1");
    e.write("b.txt", "1");
    let v1 = e.save();
    e.write("a.txt", "2");
    e.write("b.txt", "2");
    failpoint::arm("execute_item", 1, ErrorCode::Io);
    let r = e.restore(&v1, &NoProgress);
    assert_eq!(r.status, OpStatus::Incomplete, "{r:?}");
    assert_eq!((r.completed.len(), r.failed.len()), (1, 1));
    assert!(r.backup_id.is_some());
    assert!(e.incomplete());

    // 处置前不能保存
    let err = e
        .core
        .save_version(
            &SaveRequest { project_id: e.pid.clone(), request_id: new_id(), name: String::new(), note: String::new() },
            &NoProgress,
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::IncompleteOperation);
    // 但可以从安全备份找回单个文件
    let open = e.core.open_incomplete(&e.pid).unwrap().unwrap();
    assert_eq!(open.operation_id, r.operation_id);

    // 用户再修改文件后重试：重新扫描、确认并备份
    e.write("b.txt", "3");
    let plan = e.core.plan_retry(&e.pid, &r.operation_id, &NoProgress).unwrap();
    let retry = e.core.retry_operation(&e.pid, &new_id(), &r.operation_id, &plan.fingerprint, &NoProgress).unwrap();
    assert_eq!(retry.status, OpStatus::Succeeded);
    assert_eq!(e.read("b.txt").as_deref(), Some("1"));
    assert!(!e.incomplete());
    let old = e.core.operation_detail(&e.pid, &r.operation_id).unwrap();
    assert_eq!(old.summary.status, OpStatus::Incomplete);
    assert!(old.summary.resolved);
    let new = e.core.operation_detail(&e.pid, &retry.operation_id).unwrap();
    assert_eq!(new.summary.retry_of.as_deref(), Some(r.operation_id.as_str()));
}

#[test]
fn resolve_keeps_current_files() {
    let e = env();
    e.write("a.txt", "1");
    e.write("b.txt", "1");
    let v1 = e.save();
    e.write("a.txt", "2");
    e.write("b.txt", "2");
    failpoint::arm("execute_item", 1, ErrorCode::Io);
    let r = e.restore(&v1, &NoProgress);
    e.core.resolve_incomplete(&e.pid, &r.operation_id).unwrap();
    assert!(!e.incomplete());
    assert!(e.core.resolve_incomplete(&e.pid, &r.operation_id).is_err());
}

/// LFVM-AT-13：执行阶段取消，结果为“未完成”并列出已处理文件；重复提交只执行一次。
#[test]
fn cancel_during_writing_and_duplicate_request() {
    let e = env();
    for i in 0..5 {
        e.write(&format!("f{i}.txt"), "1");
    }
    let v1 = e.save();
    for i in 0..5 {
        e.write(&format!("f{i}.txt"), "2");
    }
    let mut p = at(Stage::Writing, || {});
    p.cancel_after = true;
    // 第一次报告写入进度时（尚未写任何文件）就请求取消
    let r = e.restore(&v1, &p);
    assert_eq!(r.status, OpStatus::Cancelled);
    assert!(!e.incomplete());

    let plan = e.core.plan_restore(&e.pid, &v1, &NoProgress).unwrap();
    let req = WorkspaceOpRequest {
        project_id: e.pid.clone(),
        request_id: "same".into(),
        version_id: v1.clone(),
        fingerprint: plan.fingerprint,
    };
    let a = e.core.restore_version(&req, &NoProgress).unwrap();
    let b = e.core.restore_version(&req, &NoProgress).unwrap();
    assert_eq!(a.operation_id, b.operation_id);
    assert_eq!(e.core.list_operations(&e.pid).unwrap().len(), 2);
}

/// 用户确认后文件夹又变化：指纹不一致，不写入。
#[test]
fn stale_confirmation_is_rejected() {
    let e = env();
    e.write("a.txt", "1");
    let v1 = e.save();
    e.write("a.txt", "2");
    let plan = e.core.plan_restore(&e.pid, &v1, &NoProgress).unwrap();
    e.write("new.txt", "appeared");
    let r = e
        .core
        .restore_version(
            &WorkspaceOpRequest {
                project_id: e.pid.clone(),
                request_id: new_id(),
                version_id: v1,
                fingerprint: plan.fingerprint,
            },
            &NoProgress,
        )
        .unwrap();
    assert_eq!(r.status, OpStatus::Failed);
    assert_eq!(e.read("a.txt").as_deref(), Some("2"));
}

/// LFVM-AT-03：执行阶段程序异常退出，重新打开后标为“未完成”，不自动继续写入。
#[test]
fn crash_during_execution_is_recovered_as_incomplete() {
    let e = env();
    e.write("a.txt", "1");
    let v1 = e.save();
    e.write("a.txt", "2");
    // 模拟：操作进行到写入阶段时进程消失
    let op = {
        let db = e.core.db();
        let id = insert_op(
            &db,
            &NewOp {
                project_id: &e.pid,
                request_id: "crash",
                op_type: OpType::Restore,
                target_ref: "{}".into(),
                resolved_version_id: Some(&v1),
                retry_of: None,
            },
        )
        .unwrap()
        .unwrap();
        let item = PlanItem {
            path: "a.txt".into(),
            key: "a.txt".into(),
            action: Action::Replace,
            entry_type: crate::model::EntryType::File,
            before_hash: Some(crate::hash::hash_bytes(b"2")),
            after_hash: Some(crate::hash::hash_bytes(b"1")),
            before_size: Some(1),
            after_size: Some(1),
            type_change: false,
        };
        insert_items(&db, &id, &[item]).unwrap();
        set_phase(&db, &id, Phase::Executing).unwrap();
        set_item_state(&db, &id, 0, "a.txt", ItemState::Applying, None).unwrap();
        id
    };
    e.core.open_project(&e.pid).unwrap();
    let d = e.core.operation_detail(&e.pid, &op).unwrap();
    assert_eq!(d.summary.status, OpStatus::Incomplete);
    assert_eq!(d.items[0].state, ItemState::Pending, "文件仍是原内容，记为未处理");
    assert!(e.incomplete());
    assert_eq!(e.read("a.txt").as_deref(), Some("2"), "不自动继续写入");
}

// ───────────────────────── 单文件恢复与找回 ─────────────────────────

fn file_req(e: &Env, source: SourceRef, path: &str, token: Option<String>) -> FileRestoreRequest {
    FileRestoreRequest {
        project_id: e.pid.clone(),
        request_id: new_id(),
        source,
        path: path.into(),
        confirm_token: token,
    }
}

/// AC-0015、AC-0020：原位恢复需要确认替换；被替换的原文件可从安全备份找回。
#[test]
fn single_file_restore_with_backup_and_retrieve() {
    let e = env();
    e.write("doc/报告.docx", "v1");
    let v1 = e.save();
    e.write("doc/报告.docx", "v2 edits");
    let src = SourceRef::Version { version_id: v1 };

    let check = e.core.check_file_restore(&e.pid, &src, "doc/报告.docx", &FileTarget::Original).unwrap();
    assert!(check.exists && !check.excluded);
    let err = e
        .core
        .restore_file(&file_req(&e, src.clone(), "doc/报告.docx", None), &FileTarget::Original, &NoProgress)
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NeedsConfirmation);

    let r = e
        .core
        .restore_file(
            &file_req(&e, src.clone(), "doc/报告.docx", check.confirm_token),
            &FileTarget::Original,
            &NoProgress,
        )
        .unwrap();
    assert_eq!(r.status, OpStatus::Succeeded, "{r:?}");
    assert_eq!(e.read("doc/报告.docx").as_deref(), Some("v1"));
    let backup = r.backup_id.unwrap();
    let ops = e.core.list_operations(&e.pid).unwrap();
    assert_eq!(ops[0].backup.as_ref().unwrap().reason, "恢复文件报告.docx前");

    // 从安全备份找回刚被替换的版本
    let bsrc = SourceRef::Backup { backup_id: backup };
    let check = e.core.check_file_restore(&e.pid, &bsrc, "doc/报告.docx", &FileTarget::Original).unwrap();
    let r = e
        .core
        .restore_file(&file_req(&e, bsrc, "doc/报告.docx", check.confirm_token), &FileTarget::Original, &NoProgress)
        .unwrap();
    assert_eq!(r.status, OpStatus::Succeeded);
    assert_eq!(e.read("doc/报告.docx").as_deref(), Some("v2 edits"));
}

/// 目标在确认后被修改：不替换。
#[test]
fn single_file_changed_after_confirmation() {
    let e = env();
    e.write("a.txt", "v1");
    let v1 = e.save();
    e.write("a.txt", "v2");
    let src = SourceRef::Version { version_id: v1 };
    let check = e.core.check_file_restore(&e.pid, &src, "a.txt", &FileTarget::Original).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    e.write("a.txt", "v3!");
    let err = e
        .core
        .restore_file(&file_req(&e, src, "a.txt", check.confirm_token), &FileTarget::Original, &NoProgress)
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ChangedExternally);
    assert_eq!(e.read("a.txt").as_deref(), Some("v3!"));
}

/// LFVM-AT-05：替换出错时不显示成功，原文件可从安全备份找回。
#[test]
fn single_file_replace_failure() {
    let e = env();
    e.write("a.txt", "v1");
    let v1 = e.save();
    e.write("a.txt", "v2");
    let src = SourceRef::Version { version_id: v1 };
    let check = e.core.check_file_restore(&e.pid, &src, "a.txt", &FileTarget::Original).unwrap();
    failpoint::arm("replace", 0, ErrorCode::FileInUse);
    let r = e
        .core
        .restore_file(&file_req(&e, src, "a.txt", check.confirm_token), &FileTarget::Original, &NoProgress)
        .unwrap();
    assert_eq!(r.status, OpStatus::Failed);
    assert_eq!(e.read("a.txt").as_deref(), Some("v2"));
    let files = e.core.source_files(&e.pid, &SourceRef::Backup { backup_id: r.backup_id.unwrap() }).unwrap();
    assert_eq!(files[0].path, "a.txt");
    // 不留下临时文件
    assert!(
        std::fs::read_dir(&e.work).unwrap().all(|f| !f.unwrap().file_name().to_string_lossy().starts_with(".lfvm~"))
    );
}

/// AC-0015：被排除路径不能原位覆盖，但可以另存；另存覆盖已有文件同样先备份，可按原位置找回。
#[test]
fn excluded_original_and_save_as() {
    let e = env();
    e.write("cache/x.txt", "v1");
    let v1 = e.save();
    e.exclude_dir("cache");
    let src = SourceRef::Version { version_id: v1 };
    let check = e.core.check_file_restore(&e.pid, &src, "cache/x.txt", &FileTarget::Original).unwrap();
    assert!(check.excluded);
    assert!(
        e.core
            .restore_file(
                &file_req(&e, src.clone(), "cache/x.txt", check.confirm_token),
                &FileTarget::Original,
                &NoProgress
            )
            .is_err()
    );

    let to = FileTarget::SaveAs(e.outside.clone());
    let r = e.core.restore_file(&file_req(&e, src.clone(), "cache/x.txt", None), &to, &NoProgress).unwrap();
    assert_eq!(r.status, OpStatus::Succeeded);
    assert_eq!(std::fs::read_to_string(e.outside.join("x.txt")).unwrap(), "v1");

    std::fs::write(e.outside.join("x.txt"), "outside edit").unwrap();
    let check = e.core.check_file_restore(&e.pid, &src, "cache/x.txt", &to).unwrap();
    assert!(check.exists);
    let r = e.core.restore_file(&file_req(&e, src, "cache/x.txt", check.confirm_token), &to, &NoProgress).unwrap();
    assert_eq!(r.status, OpStatus::Succeeded);
    let bsrc = SourceRef::Backup { backup_id: r.backup_id.unwrap() };
    let check = e.core.check_file_restore(&e.pid, &bsrc, "x.txt", &FileTarget::Original).unwrap();
    assert_eq!(Path::new(&check.target_path), crate::paths::canonical(&e.outside).unwrap().join("x.txt"));
    e.core.restore_file(&file_req(&e, bsrc, "x.txt", check.confirm_token), &FileTarget::Original, &NoProgress).unwrap();
    assert_eq!(std::fs::read_to_string(e.outside.join("x.txt")).unwrap(), "outside edit");
}

#[test]
fn save_as_rejects_history_store() {
    let e = env();
    e.write("a.txt", "v1");
    let v1 = e.save();
    let err = e
        .core
        .check_file_restore(
            &e.pid,
            &SourceRef::Version { version_id: v1 },
            "a.txt",
            &FileTarget::SaveAs(e.core.data_dir().to_path_buf()),
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::DirectoryOverlap);
}
