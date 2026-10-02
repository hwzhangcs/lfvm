//! 长任务的进度报告与取消（LFVM-P-10：进度至少每秒更新，取消后 1 秒内有反馈）。

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use crate::error::{CoreError, CoreResult, ErrorCode};

/// 任务所处阶段，界面据此显示“正在扫描”“正在保存文件”等。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// 列出文件夹内容
    Listing,
    /// 计算内容摘要
    Hashing,
    /// 写入历史存储
    Storing,
    /// 再次核对工作区
    Verifying,
    /// 登记到数据库
    Committing,
}

/// 由调用方实现：桌面外壳把进度通过 Channel 推给界面；测试中用 [`NoProgress`]。
/// 实现者应自行节流，核心库可能频繁调用 `report`。
pub trait Progress: Sync {
    fn report(&self, stage: Stage, done: u64, total: u64);
    fn is_cancelled(&self) -> bool;

    /// 已请求取消时返回 `Cancelled` 错误，供长循环中调用。
    fn check(&self) -> CoreResult<()> {
        if self.is_cancelled() {
            Err(CoreError::new(ErrorCode::Cancelled, "操作已取消"))
        } else {
            Ok(())
        }
    }
}

pub struct NoProgress;

impl Progress for NoProgress {
    fn report(&self, _: Stage, _: u64, _: u64) {}
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// 可在其他线程置位的取消标记。
#[derive(Default)]
pub struct CancelFlag(AtomicBool);

impl CancelFlag {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}
