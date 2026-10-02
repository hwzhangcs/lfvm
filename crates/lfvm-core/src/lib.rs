//! LFVM 业务核心。
//!
//! 本 crate 不依赖 Tauri：所有功能都可以在测试、性能脚本和稳定性脚本中直接调用，
//! 桌面外壳只负责把这些功能暴露为界面命令。

pub mod changes;
pub mod compare;
pub mod content;
pub mod db;
pub mod error;
pub mod exclude;
pub mod glob;
pub mod hash;
pub mod history;
pub mod model;
pub mod paths;
pub mod progress;
pub mod project;
pub mod scan;
pub mod store;
pub mod version;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::Connection;

pub use error::{CoreError, CoreResult, ErrorCode};

/// 历史存储目录内的布局：
///
/// ```text
/// <data_dir>/lfvm.db                     元数据（SQLite）
/// <data_dir>/projects/<project_id>/      各项目的内容对象、缩略图、临时文件
/// <data_dir>/logs/                       日志（只记录错误码和路径）
/// ```
pub struct Core {
    data_dir: PathBuf,
    conn: Mutex<Connection>,
    /// 正在进行写操作的项目。同一项目的保存、规则修改、恢复、切换和清理不得同时执行（LFVM-Q-05）。
    busy: Mutex<HashSet<String>>,
}

/// 写操作期间持有；释放时自动解除项目的写锁。
pub(crate) struct WriteGuard<'a> {
    core: &'a Core,
    project_id: String,
}

impl Drop for WriteGuard<'_> {
    fn drop(&mut self) {
        self.core.busy.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.project_id);
    }
}

impl Core {
    /// 打开（必要时创建）历史存储目录。
    pub fn open(data_dir: impl Into<PathBuf>) -> CoreResult<Self> {
        let data_dir = data_dir.into();
        for dir in [data_dir.clone(), data_dir.join("projects"), data_dir.join("logs")] {
            std::fs::create_dir_all(&dir).map_err(|e| CoreError::io(e, &dir))?;
        }
        let data_dir = paths::canonical(&data_dir)?;
        let conn = db::open(&data_dir.join("lfvm.db"))?;
        Ok(Self { data_dir, conn: Mutex::new(conn), busy: Mutex::new(HashSet::new()) })
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// 某个项目的内容存储目录。
    pub fn project_store_dir(&self, project_id: &str) -> PathBuf {
        self.data_dir.join("projects").join(project_id)
    }

    pub(crate) fn db(&self) -> MutexGuard<'_, Connection> {
        // 持锁线程 panic 后连接本身仍可用；事务未提交的部分已由 SQLite 回滚。
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn begin_write(&self, project_id: &str) -> CoreResult<WriteGuard<'_>> {
        let mut busy = self.busy.lock().unwrap_or_else(|e| e.into_inner());
        if !busy.insert(project_id.to_owned()) {
            return Err(CoreError::new(ErrorCode::Busy, "这个项目正在进行其他操作，请等它完成后再试"));
        }
        Ok(WriteGuard { core: self, project_id: project_id.to_owned() })
    }
}

/// 当前 UTC 时间（毫秒）。
pub(crate) fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// 新的唯一标识（ULID，按时间有序）。
pub fn new_id() -> String {
    ulid::Ulid::generate().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_layout() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open(dir.path().join("LFVM")).unwrap();
        assert!(core.data_dir().join("lfvm.db").is_file());
        assert!(core.data_dir().join("projects").is_dir());
        assert!(core.project_store_dir("p1").ends_with("projects/p1"));
    }
}
