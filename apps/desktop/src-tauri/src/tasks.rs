//! 长任务的进度推送与取消。
//!
//! 界面为每个长任务生成 task_id 并传入一个 Channel；核心库通过 [`Progress`] 报告进度，
//! 这里节流后经 Channel 推给界面（LFVM-P-10：至少每秒一次）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lfvm_core::progress::{CancelFlag, Progress, Stage};
use serde::Serialize;
use tauri::ipc::Channel;

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct ProgressEvent {
    pub stage: Stage,
    /// 已完成量（文件数或字节数，视阶段而定）；total 为 0 表示总量未知。
    #[specta(type = specta_typescript::Number)]
    pub done: u64,
    #[specta(type = specta_typescript::Number)]
    pub total: u64,
}

const MIN_INTERVAL: Duration = Duration::from_millis(200);

pub struct ChannelProgress {
    channel: Channel<ProgressEvent>,
    cancel: Arc<CancelFlag>,
    last: Mutex<(Instant, Option<Stage>)>,
}

impl ChannelProgress {
    pub fn new(channel: Channel<ProgressEvent>, cancel: Arc<CancelFlag>) -> Self {
        Self { channel, cancel, last: Mutex::new((Instant::now() - MIN_INTERVAL, None)) }
    }
}

impl Progress for ChannelProgress {
    fn report(&self, stage: Stage, done: u64, total: u64) {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        // 阶段切换和完成时立即推送，其余按间隔节流
        let force = last.1 != Some(stage) || (total > 0 && done >= total);
        if !force && last.0.elapsed() < MIN_INTERVAL {
            return;
        }
        *last = (Instant::now(), Some(stage));
        let _ = self.channel.send(ProgressEvent { stage, done, total });
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_set()
    }
}

/// 进行中的任务，用于响应“取消”。
#[derive(Default)]
pub struct TaskRegistry(Mutex<HashMap<String, Arc<CancelFlag>>>);

impl TaskRegistry {
    pub fn start(&self, task_id: &str) -> TaskHandle<'_> {
        let flag = Arc::new(CancelFlag::default());
        self.0.lock().unwrap_or_else(|e| e.into_inner()).insert(task_id.to_owned(), flag.clone());
        TaskHandle { registry: self, task_id: task_id.to_owned(), flag }
    }

    pub fn cancel(&self, task_id: &str) -> bool {
        match self.0.lock().unwrap_or_else(|e| e.into_inner()).get(task_id) {
            Some(f) => {
                f.cancel();
                true
            }
            None => false,
        }
    }
}

pub struct TaskHandle<'a> {
    registry: &'a TaskRegistry,
    task_id: String,
    pub flag: Arc<CancelFlag>,
}

impl Drop for TaskHandle<'_> {
    fn drop(&mut self) {
        self.registry.0.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.task_id);
    }
}
