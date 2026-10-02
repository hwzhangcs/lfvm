//! 统一错误类型。每个错误带错误码（LFVM-O-02），界面据此显示原因和下一步操作。

use serde::Serialize;

/// 错误码。新增时只在末尾追加，界面文案在前端按错误码维护。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// 输入不符合格式（名称长度、必填项等）。
    InvalidInput,
    /// 路径为空、为绝对路径、含“..”或无法解析。
    InvalidPath,
    /// 路径超出项目、历史存储或用户选择的范围。
    PathOutOfScope,
    /// 路径超出系统支持的长度。
    PathTooLong,
    /// 遇到未排除的链接对象（符号链接、目录联接）。
    LinkNotFollowed,
    /// 仅存于云端的占位文件。
    CloudPlaceholder,
    /// 磁盘根目录或系统目录。
    ProtectedDirectory,
    /// 与已登记项目或历史存储目录相同或相互包含。
    DirectoryOverlap,
    NotFound,
    AlreadyExists,
    PermissionDenied,
    /// 文件被其他程序占用。
    FileInUse,
    /// 磁盘空间不足。
    NoSpace,
    /// 文件在操作过程中被外部修改。
    ChangedExternally,
    /// 同一项目已有写操作在进行。
    Busy,
    Cancelled,
    /// 项目存在未处置的未完成操作（SRS 6.1.1）。
    IncompleteOperation,
    /// 历史内容校验失败。
    ContentCorrupted,
    Database,
    Io,
    Internal,
    /// 没有需要保存的变化。
    NothingToSave,
    /// 版本或安全备份的内容已清理。
    ContentCleared,
    /// 保留排除内容与目标目录结构无法同时成立（规则 R-05）。
    StructureConflict,
    /// 目标位置已有文件，需要用户确认替换。
    NeedsConfirmation,
}

#[derive(Debug, Clone, thiserror::Error, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[error("{message}")]
pub struct CoreError {
    pub code: ErrorCode,
    /// 面向用户的中文说明。
    pub message: String,
    /// 涉及的路径（如有）。日志只记录错误码和路径，不记录文件正文（LFVM-Q-08）。
    pub path: Option<String>,
}

pub type CoreResult<T> = Result<T, CoreError>;

impl CoreError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), path: None }
    }

    pub fn with_path(mut self, path: impl AsRef<std::path::Path>) -> Self {
        self.path = Some(path.as_ref().to_string_lossy().into_owned());
        self
    }

    pub fn io(err: std::io::Error, path: impl AsRef<std::path::Path>) -> Self {
        Self::from(err).with_path(path)
    }
}

impl From<std::io::Error> for CoreError {
    fn from(err: std::io::Error) -> Self {
        use std::io::ErrorKind as K;
        let (code, message) = match err.kind() {
            K::NotFound => (ErrorCode::NotFound, "文件或文件夹不存在"),
            K::PermissionDenied => (ErrorCode::PermissionDenied, "没有访问权限"),
            K::AlreadyExists => (ErrorCode::AlreadyExists, "目标已存在"),
            K::StorageFull | K::QuotaExceeded => (ErrorCode::NoSpace, "磁盘空间不足"),
            K::InvalidFilename => (ErrorCode::PathTooLong, "路径过长或名称无效"),
            K::ResourceBusy => (ErrorCode::FileInUse, "文件正被其他程序使用"),
            _ => (ErrorCode::Io, "读写文件时出错"),
        };
        // Windows 共享冲突（ERROR_SHARING_VIOLATION=32、ERROR_LOCK_VIOLATION=33）
        if cfg!(windows) && matches!(err.raw_os_error(), Some(32) | Some(33)) {
            return Self::new(ErrorCode::FileInUse, "文件正被其他程序使用");
        }
        Self::new(code, format!("{message}（{err}）"))
    }
}

impl From<rusqlite::Error> for CoreError {
    fn from(err: rusqlite::Error) -> Self {
        Self::new(ErrorCode::Database, format!("历史记录数据库出错（{err}）"))
    }
}
