//! 逐项写入工作区（SRS 3.3.2.5 第 5 步、LFVM-Q-03）。
//!
//! 每项写入前确认文件在备份后未被外部修改；新内容先写到同目录的临时文件并校验，再原子替换。
//! 任一项失败立即停止，其余项保持“未处理”。

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use lfvm_platform::EntryKind;

use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::exclude::INTERNAL_TMP_PREFIX;
use crate::hash::{hash_file, hash_reader};
use crate::model::EntryType;
use crate::ops::failpoint;
use crate::ops::plan::{Action, PlanItem};
use crate::ops::{ItemState, set_item_state};
use crate::paths::RelPath;
use crate::progress::{Progress, Stage};
use crate::store::ObjectStore;
use crate::{Core, new_id};

pub(crate) enum ExecOutcome {
    Completed,
    /// 中途停止。`touched` 表示是否已有文件被改动（决定记为“失败”还是“未完成”）。
    Stopped { error: Option<CoreError>, touched: bool },
}

impl Core {
    pub(crate) fn execute_items(
        &self,
        op_id: &str,
        root: &Path,
        store: &ObjectStore,
        items: &[(usize, &PlanItem)],
        progress: &dyn Progress,
    ) -> ExecOutcome {
        let mut safe_dirs = HashSet::new();
        let total = items.len() as u64;
        progress.report(Stage::Writing, 0, total);
        let mut touched = false;
        for (n, (index, it)) in items.iter().enumerate() {
            // 取消请求在当前文件处理完后生效（SRS 6.1.1）
            if progress.is_cancelled() {
                return ExecOutcome::Stopped { error: None, touched };
            }
            let _ = set_item_state(&self.db(), op_id, *index, &it.key, ItemState::Applying, None);
            let r = failpoint::hit("execute_item").and_then(|_| apply_item(root, store, it, &mut safe_dirs));
            match r {
                Ok(()) => {
                    touched = true;
                    let _ = set_item_state(&self.db(), op_id, *index, &it.key, ItemState::Done, None);
                }
                Err(e) => {
                    let _ = set_item_state(&self.db(), op_id, *index, &it.key, ItemState::Failed, Some(&e));
                    return ExecOutcome::Stopped { error: Some(e), touched };
                }
            }
            progress.report(Stage::Writing, n as u64 + 1, total);
        }
        ExecOutcome::Completed
    }
}

fn apply_item(root: &Path, store: &ObjectStore, it: &PlanItem, safe_dirs: &mut HashSet<PathBuf>) -> CoreResult<()> {
    let rel = RelPath::parse(&it.path)?;
    let abs = rel.to_path(root);
    check_no_links(root, &rel, safe_dirs)?;
    match (it.action, it.entry_type) {
        (Action::Delete, EntryType::File) => {
            verify_unchanged(&abs, it.before_hash.as_deref())?;
            std::fs::remove_file(&abs).map_err(|e| CoreError::io(e, &abs))
        }
        (Action::Delete, EntryType::Directory) => match std::fs::remove_dir(&abs) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(CoreError::io(e, &abs)),
            _ => Ok(()),
        },
        (Action::Create, EntryType::Directory) => {
            if abs.is_dir() {
                Ok(())
            } else {
                std::fs::create_dir_all(&abs).map_err(|e| CoreError::io(e, &abs))
            }
        }
        (Action::Create, EntryType::File) => {
            if std::fs::symlink_metadata(&abs).is_ok() {
                return Err(changed(&abs, "目标位置出现了新的文件，为避免覆盖，操作已停止"));
            }
            write_object(store, it.after_hash.as_deref(), &abs, None)
        }
        (Action::Replace, EntryType::File) => write_object(store, it.after_hash.as_deref(), &abs, it.before_hash.as_deref()),
        _ => Ok(()),
    }
}

fn changed(path: &Path, msg: &str) -> CoreError {
    CoreError::new(ErrorCode::ChangedExternally, msg).with_path(path)
}

/// 确认文件仍是备份时的内容（LFVM-AT-02）。
pub(crate) fn verify_unchanged(abs: &Path, expected: Option<&str>) -> CoreResult<()> {
    let Some(expected) = expected else { return Ok(()) };
    match hash_file(abs) {
        Ok((h, _)) if h == expected => Ok(()),
        Ok(_) => Err(changed(abs, "文件在备份后被修改，为避免覆盖新的修改，操作已停止")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(changed(abs, "文件在备份后被移动或删除，操作已停止"))
        }
        Err(e) => Err(CoreError::io(e, abs)),
    }
}

/// 不跟随链接对象越出范围（LFVM-Q-07）：目标的各级上级目录都不能是链接。
pub(crate) fn check_no_links(root: &Path, rel: &RelPath, safe: &mut HashSet<PathBuf>) -> CoreResult<()> {
    let mut cur = root.to_path_buf();
    let comps: Vec<&str> = rel.components().collect();
    for c in &comps[..comps.len().saturating_sub(1)] {
        cur.push(c);
        if safe.contains(&cur) {
            continue;
        }
        match std::fs::symlink_metadata(&cur) {
            Ok(m) if lfvm_platform::classify(&m) == EntryKind::Link => {
                return Err(CoreError::new(ErrorCode::LinkNotFollowed, "路径中有快捷链接，系统不会经由它写入").with_path(&cur));
            }
            Ok(_) => {
                safe.insert(cur.clone());
            }
            Err(_) => break, // 尚不存在的目录稍后由本系统创建
        }
    }
    Ok(())
}

/// 把内容对象写到 `dst`：先写同目录临时文件并校验，再确认目标未变化，最后原子替换。
pub(crate) fn write_object(store: &ObjectStore, hash: Option<&str>, dst: &Path, before: Option<&str>) -> CoreResult<()> {
    let hash = hash.ok_or_else(|| CoreError::new(ErrorCode::Internal, "缺少目标内容"))?;
    let tmp = prepare_temp(store, hash, dst)?;
    let r = (|| {
        verify_unchanged(dst, before)?;
        failpoint::hit("replace")?;
        lfvm_platform::atomic_replace(&tmp, dst).map_err(|e| CoreError::io(e, dst))
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

/// 在目标同目录写出临时文件并校验摘要，返回临时文件路径。
pub(crate) fn prepare_temp(store: &ObjectStore, hash: &str, dst: &Path) -> CoreResult<PathBuf> {
    let parent = dst.parent().ok_or_else(|| CoreError::new(ErrorCode::InvalidPath, "无效的目标位置"))?;
    std::fs::create_dir_all(parent).map_err(|e| CoreError::io(e, parent))?;
    let tmp = parent.join(format!("{INTERNAL_TMP_PREFIX}{}.tmp", new_id()));
    let r = (|| {
        let src = store.open(hash)?;
        let out = File::create(&tmp).map_err(|e| CoreError::io(e, &tmp))?;
        let mut w = BufWriter::new(out);
        let (got, _) = hash_reader(src, Some(&mut w)).map_err(|e| CoreError::io(e, &tmp))?;
        w.flush().map_err(|e| CoreError::io(e, &tmp))?;
        drop(w);
        if got != hash {
            return Err(CoreError::new(ErrorCode::ContentCorrupted, "历史内容已损坏（校验不一致），没有写入").with_path(dst));
        }
        Ok(())
    })();
    match r {
        Ok(()) => Ok(tmp),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}
