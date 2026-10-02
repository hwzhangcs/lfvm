//! 展开与导出（SRS 3.3.4.3、3.3.4.4、LFVM-C-06）。
//!
//! 两者都把某个版本（或方案的当前末端）按保存时的清单输出为普通文件夹，不受当前排除规则影响，
//! 不登记为项目，也不回写历史：
//! - 目标必须是新建或空文件夹，且不能与任何项目文件夹或历史存储位置相同或相互包含；
//! - 写入时不覆盖任何已存在的文件，逐项校验；失败时只清理本次创建的文件；
//! - 展开面向临时查看或并排比较，可同时展开两个方案（允许一个成功一个失败，即“部分完成”）；
//!   导出面向交付，只有全部文件校验通过才报告完成。

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use lfvm_platform::EntryKind;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::Core;
use crate::changes::load_manifest;
use crate::content::{SourceRef, check_source};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::hash::hash_reader;
use crate::model::EntryType;
use crate::ops::backup::ensure_space;
use crate::ops::restore::TargetRef;
use crate::ops::{ItemFailure, NewOp, OpStatus, OpType, finish_op, insert_op};
use crate::paths::{self, RelPath};
use crate::progress::{Progress, Stage};
use crate::project::{load_project, version_brief};
use crate::scheme::scheme_head;
use crate::store::ObjectStore;

/// 输出哪个版本：直接指定版本，或某个方案（以其当前末端为准）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutputSource {
    Version { version_id: String },
    Scheme { scheme_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    Expand,
    Export,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct OutputResult {
    pub operation_id: String,
    pub status: OpStatus,
    pub message: String,
    /// 输出文件夹。
    pub path: String,
    pub source_label: String,
    pub files_written: u32,
    pub total_files: u32,
    pub failed: Vec<ItemFailure>,
}

/// 输出目标：所选位置下的一个新文件夹。
#[derive(Debug, Clone)]
pub struct OutputTarget {
    pub parent: PathBuf,
    pub folder_name: String,
}

/// 默认文件夹名：“项目名_版本名”（版本没有名称时用 V 序号）。
pub fn default_folder_name(project_name: &str, seq: u32, version_name: &str) -> String {
    let v = if version_name.trim().is_empty() { format!("V{seq}") } else { version_name.trim().to_owned() };
    let raw = format!("{project_name}_{v}");
    // 去掉当前平台不允许的字符
    raw.chars()
        .map(|c| if lfvm_platform::validate_component(&c.to_string()).is_ok() { c } else { '_' })
        .collect::<String>()
        .trim_end_matches(['.', ' '])
        .to_owned()
}

impl Core {
    fn resolve_output_source(&self, project_id: &str, source: &OutputSource) -> CoreResult<(String, String)> {
        let db = self.db();
        let version = match source {
            OutputSource::Version { version_id } => version_id.clone(),
            OutputSource::Scheme { scheme_id } => scheme_head(&db, project_id, scheme_id)?.1,
        };
        check_source(&db, project_id, &SourceRef::Version { version_id: version.clone() })?;
        let b = version_brief(&db, &version)?;
        let label = match source {
            OutputSource::Version { .. } => format!("V{} {}", b.seq, b.name).trim().to_owned(),
            OutputSource::Scheme { scheme_id } => {
                format!("方案“{}”（V{}）", scheme_head(&db, project_id, scheme_id)?.0, b.seq)
            }
        };
        Ok((version, label))
    }

    /// 建议的输出文件夹名称。
    pub fn suggest_output_name(&self, project_id: &str, source: &OutputSource) -> CoreResult<String> {
        let (version, _) = self.resolve_output_source(project_id, source)?;
        let db = self.db();
        let p = load_project(&db, project_id)?;
        let b = version_brief(&db, &version)?;
        Ok(default_folder_name(&p.name, b.seq, &b.name))
    }

    /// 检查输出目标：新建或空文件夹；不与项目文件夹、历史存储位置重叠；不是链接。
    fn check_output_target(&self, target: &OutputTarget) -> CoreResult<PathBuf> {
        lfvm_platform::validate_component(&target.folder_name)
            .map_err(|_| CoreError::new(ErrorCode::InvalidInput, "文件夹名称为空或含有不能使用的字符"))?;
        let meta = std::fs::symlink_metadata(&target.parent).map_err(|e| CoreError::io(e, &target.parent))?;
        if lfvm_platform::classify(&meta) != EntryKind::Dir {
            return Err(
                CoreError::new(ErrorCode::InvalidInput, "请选择一个文件夹作为输出位置").with_path(&target.parent)
            );
        }
        let parent = paths::canonical(&target.parent)?;
        let dest = parent.join(&target.folder_name);
        let policy = lfvm_platform::default_case_policy();
        if paths::same_or_nested(&dest, self.data_dir(), policy) {
            return Err(CoreError::new(ErrorCode::DirectoryOverlap, "不能输出到本软件的历史存储位置").with_path(&dest));
        }
        for p in self.list_projects()? {
            if paths::same_or_nested(&dest, Path::new(&p.root_path), policy) {
                return Err(CoreError::new(
                    ErrorCode::DirectoryOverlap,
                    format!("输出位置与项目“{}”的文件夹相同或相互包含，请改选", p.name),
                )
                .with_path(&dest));
            }
        }
        match std::fs::symlink_metadata(&dest) {
            Err(_) => {}
            Ok(m) if lfvm_platform::classify(&m) == EntryKind::Dir => {
                let empty = std::fs::read_dir(&dest).map_err(|e| CoreError::io(e, &dest))?.next().is_none();
                if !empty {
                    return Err(CoreError::new(
                        ErrorCode::AlreadyExists,
                        "目标文件夹不是空的，请改选或换一个名称（不会合并或覆盖）",
                    )
                    .with_path(&dest));
                }
            }
            Ok(_) => {
                return Err(
                    CoreError::new(ErrorCode::AlreadyExists, "目标位置已有同名文件，请换一个名称").with_path(&dest)
                );
            }
        }
        Ok(dest)
    }

    /// 展开或导出一个版本 / 方案。
    pub fn output_version(
        &self,
        project_id: &str,
        request_id: &str,
        kind: OutputKind,
        source: &OutputSource,
        target: &OutputTarget,
        progress: &dyn Progress,
    ) -> CoreResult<OutputResult> {
        let (version, label) = self.resolve_output_source(project_id, source)?;
        let dest = self.check_output_target(target)?;
        let _lease = self.lease(&version);
        let op_type = if kind == OutputKind::Expand { OpType::Expand } else { OpType::Export };
        let op_id = {
            let db = self.db();
            match insert_op(
                &db,
                &NewOp {
                    project_id,
                    request_id,
                    op_type,
                    target_ref: TargetRef {
                        label: label.clone(),
                        scheme: None,
                        path: Some(dest.to_string_lossy().into_owned()),
                    }
                    .to_json(),
                    resolved_version_id: Some(&version),
                    retry_of: None,
                },
            )? {
                Ok(id) => id,
                Err(_) => return Err(CoreError::new(ErrorCode::Busy, "这次请求已经处理过")),
            }
        };
        let manifest = load_manifest(&self.db(), &version)?;
        let mut entries: Vec<_> = manifest.values().collect();
        entries.sort_by(|a, b| a.rel.cmp(&b.rel));
        let total_files = entries.iter().filter(|e| e.entry_type == EntryType::File).count() as u32;
        let store = ObjectStore::new(&self.project_store_dir(project_id));

        let mut created: Vec<PathBuf> = Vec::new();
        let mut written = 0u32;
        let mut failure: Option<ItemFailure> = None;
        let result = (|| -> CoreResult<()> {
            // 写入前检查：名称在当前系统上可用、目标盘空间足够
            for e in &entries {
                if let Some(bad) =
                    RelPath::parse(&e.rel)?.components().find(|c| lfvm_platform::validate_component(c).is_err())
                {
                    return Err(CoreError::new(ErrorCode::InvalidPath, format!("名称“{bad}”在当前系统上不能使用"))
                        .with_path(&e.rel));
                }
            }
            let bytes: u64 = entries.iter().filter_map(|e| e.size).sum();
            ensure_space(dest.parent().unwrap_or(&dest), bytes, "输出位置")?;

            if !dest.exists() {
                std::fs::create_dir(&dest).map_err(|e| CoreError::io(e, &dest))?;
                created.push(dest.clone());
            }
            let total_bytes = bytes;
            let mut done_bytes = 0u64;
            progress.report(Stage::Writing, 0, total_bytes);
            for e in &entries {
                progress.check()?;
                let rel = RelPath::parse(&e.rel)?;
                let abs = rel.to_path(&dest);
                match e.entry_type {
                    EntryType::Directory => {
                        std::fs::create_dir(&abs).map_err(|err| CoreError::io(err, &abs))?;
                        created.push(abs);
                    }
                    EntryType::File => {
                        let hash = e.hash.as_deref().unwrap_or_default();
                        // create_new：绝不覆盖已存在的文件
                        let out = OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&abs)
                            .map_err(|err| CoreError::io(err, &abs))?;
                        created.push(abs.clone());
                        let r = (|| {
                            let src = store.open(hash)?;
                            let mut w = BufWriter::new(out);
                            let (got, _) = hash_reader(src, Some(&mut w)).map_err(|err| CoreError::io(err, &abs))?;
                            w.flush().map_err(|err| CoreError::io(err, &abs))?;
                            if got != hash {
                                return Err(CoreError::new(
                                    ErrorCode::ContentCorrupted,
                                    "历史内容已损坏（校验不一致）",
                                ));
                            }
                            Ok(())
                        })();
                        if let Err(err) = r {
                            failure = Some(ItemFailure { path: e.rel.clone(), message: err.message.clone() });
                            return Err(err);
                        }
                        written += 1;
                        done_bytes += e.size.unwrap_or(0);
                        progress.report(Stage::Writing, done_bytes, total_bytes);
                    }
                }
            }
            if let Some(f) = created.iter().find(|p| p.is_file()) {
                let _ = File::open(f).and_then(|f| f.sync_all());
            }
            Ok(())
        })();

        let (status, message) = match &result {
            Ok(()) => (
                OpStatus::Succeeded,
                if kind == OutputKind::Export { "导出完成".to_owned() } else { "展开完成".to_owned() },
            ),
            Err(e) => {
                // 只清理本次创建的文件和文件夹（由深到浅）
                for p in created.iter().rev() {
                    let _ = if p.is_dir() { std::fs::remove_dir(p) } else { std::fs::remove_file(p) };
                }
                let what = if kind == OutputKind::Export { "导出" } else { "展开" };
                if e.code == ErrorCode::Cancelled {
                    (OpStatus::Cancelled, format!("已取消{what}，已清理本次创建的文件"))
                } else {
                    (
                        OpStatus::Failed,
                        format!(
                            "{what}没有完成：{}。已写入 {written}/{total_files} 个文件，已清理本次创建的文件",
                            e.message
                        ),
                    )
                }
            }
        };
        finish_op(&self.db(), &op_id, status, &message)?;
        if failure.is_none()
            && let Err(e) = &result
            && e.code != ErrorCode::Cancelled
        {
            failure = Some(ItemFailure { path: e.path.clone().unwrap_or_default(), message: e.message.clone() });
        }
        Ok(OutputResult {
            operation_id: op_id,
            status,
            message,
            path: dest.to_string_lossy().into_owned(),
            source_label: label,
            files_written: written,
            total_files,
            failed: failure.into_iter().collect(),
        })
    }

    /// 展开、导出操作的输出文件夹（用于“打开文件夹”）。
    pub fn output_path(&self, project_id: &str, operation_id: &str) -> CoreResult<PathBuf> {
        let target: String = self.db().query_row(
            "SELECT target_ref FROM operations WHERE project_id = ?1 AND operation_id = ?2 AND type IN ('expand', 'export')",
            params![project_id, operation_id],
            |r| r.get(0),
        )?;
        TargetRef::parse(&target)
            .path
            .map(PathBuf::from)
            .ok_or_else(|| CoreError::new(ErrorCode::NotFound, "找不到输出文件夹"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::NoProgress;
    use crate::version::SaveRequest;

    struct Env {
        tmp: tempfile::TempDir,
        core: Core,
        work: PathBuf,
        pid: String,
    }

    fn env() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let core = Core::open(tmp.path().join("data")).unwrap();
        let work = tmp.path().join("作业");
        std::fs::create_dir_all(work.join("sub/empty")).unwrap();
        std::fs::write(work.join("a.txt"), "A").unwrap();
        std::fs::write(work.join("sub/b.txt"), "B").unwrap();
        std::fs::write(work.join("zero"), "").unwrap();
        let pid = core.add_project(&work, None).unwrap().project_id;
        Env { tmp, core, work, pid }
    }

    fn save(e: &Env, name: &str) -> String {
        e.core
            .save_version(
                &SaveRequest {
                    project_id: e.pid.clone(),
                    request_id: crate::new_id(),
                    name: name.into(),
                    note: String::new(),
                },
                &NoProgress,
            )
            .unwrap()
            .version_id
    }

    fn out(e: &Env, kind: OutputKind, src: OutputSource, name: &str) -> CoreResult<OutputResult> {
        e.core.output_version(
            &e.pid,
            &crate::new_id(),
            kind,
            &src,
            &OutputTarget { parent: e.tmp.path().to_path_buf(), folder_name: name.into() },
            &NoProgress,
        )
    }

    /// AC-0028、AC-0030：展开/导出的内容与版本清单一致，含空目录和零字节文件，不含系统内部文件；
    /// 修改副本不影响项目。
    #[test]
    fn export_matches_manifest() {
        let e = env();
        let v1 = save(&e, "交作业版");
        assert_eq!(
            e.core.suggest_output_name(&e.pid, &OutputSource::Version { version_id: v1.clone() }).unwrap(),
            "作业_交作业版"
        );
        let r = out(&e, OutputKind::Export, OutputSource::Version { version_id: v1 }, "导出").unwrap();
        assert_eq!(r.status, OpStatus::Succeeded);
        assert_eq!((r.files_written, r.total_files), (3, 3));
        let d = e.tmp.path().join("导出");
        assert_eq!(std::fs::read_to_string(d.join("sub/b.txt")).unwrap(), "B");
        assert!(d.join("sub/empty").is_dir());
        assert_eq!(std::fs::metadata(d.join("zero")).unwrap().len(), 0);
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 3, "只有用户文件");
        std::fs::write(d.join("a.txt"), "changed copy").unwrap();
        assert_eq!(std::fs::read_to_string(e.work.join("a.txt")).unwrap(), "A");
        assert_eq!(e.core.output_path(&e.pid, &r.operation_id).unwrap(), crate::paths::canonical(&d).unwrap());
    }

    /// AC-0029、AC-0030：目标非空、与项目或历史存储重叠时拒绝；两个方案分别展开。
    #[test]
    fn rejects_bad_targets_and_expands_two_schemes() {
        let e = env();
        let v1 = save(&e, "");
        std::fs::write(e.work.join("a.txt"), "A2").unwrap();
        let v2 = save(&e, "");
        let s1 = e.core.create_scheme(&e.pid, &v1, "一").unwrap();
        let s2 = e.core.create_scheme(&e.pid, &v2, "二").unwrap();

        std::fs::create_dir_all(e.tmp.path().join("full")).unwrap();
        std::fs::write(e.tmp.path().join("full/x"), "x").unwrap();
        let src = OutputSource::Scheme { scheme_id: s1.scheme_id.clone() };
        assert_eq!(out(&e, OutputKind::Expand, src.clone(), "full").unwrap_err().code, ErrorCode::AlreadyExists);
        assert_eq!(out(&e, OutputKind::Expand, src.clone(), "作业").unwrap_err().code, ErrorCode::DirectoryOverlap);
        let bad = e.core.output_version(
            &e.pid,
            &crate::new_id(),
            OutputKind::Expand,
            &src,
            &OutputTarget { parent: e.work.clone(), folder_name: "inside".into() },
            &NoProgress,
        );
        assert_eq!(bad.unwrap_err().code, ErrorCode::DirectoryOverlap);

        let a = out(&e, OutputKind::Expand, src, "方案一").unwrap();
        let b = out(&e, OutputKind::Expand, OutputSource::Scheme { scheme_id: s2.scheme_id }, "方案二").unwrap();
        assert_eq!((a.status, b.status), (OpStatus::Succeeded, OpStatus::Succeeded));
        assert_eq!(std::fs::read_to_string(e.tmp.path().join("方案一/a.txt")).unwrap(), "A");
        assert_eq!(std::fs::read_to_string(e.tmp.path().join("方案二/a.txt")).unwrap(), "A2");
    }

    /// 写入失败时不报告完成，只清理本次创建的文件。
    #[test]
    fn failure_cleans_up_only_created_files() {
        let e = env();
        let v1 = save(&e, "");
        // 破坏一个内容对象
        let h = crate::hash::hash_bytes(b"B");
        std::fs::write(ObjectStore::new(&e.core.project_store_dir(&e.pid)).object_path(&h).unwrap(), "corrupt")
            .unwrap();
        std::fs::create_dir_all(e.tmp.path().join("pre")).unwrap();
        let r = out(&e, OutputKind::Export, OutputSource::Version { version_id: v1 }, "pre").unwrap();
        assert_eq!(r.status, OpStatus::Failed);
        assert_eq!(r.failed.len(), 1);
        assert!(e.tmp.path().join("pre").is_dir(), "用户事先建好的空文件夹保留");
        assert_eq!(std::fs::read_dir(e.tmp.path().join("pre")).unwrap().count(), 0);
    }
}
