//! 错误日志（LFVM-Q-08）：只记录时间、错误码和路径，不记录文件正文；超过 5 MB 时轮换。

use std::io::Write;

use crate::error::CoreError;
use crate::{Core, now_ms};

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

impl Core {
    /// 追加一条错误记录。写日志失败不影响原操作。
    pub fn log_error(&self, context: &str, err: &CoreError) {
        let dir = self.data_dir().join("logs");
        let path = dir.join("lfvm.log");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
            let _ = std::fs::rename(&path, dir.join("lfvm.log.1"));
        }
        let code = serde_json::to_value(err.code).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
        let line = format!(
            "{}\t{}\t{}\t{}\n",
            now_ms(),
            context.replace(['\t', '\n'], " "),
            code,
            err.path.as_deref().unwrap_or("").replace(['\t', '\n'], " ")
        );
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::error::{CoreError, ErrorCode};

    #[test]
    fn logs_code_and_path_only() {
        let d = tempfile::tempdir().unwrap();
        let core = crate::Core::open(d.path()).unwrap();
        let e = CoreError::new(ErrorCode::FileInUse, "文件内容：机密正文").with_path("docs/a.txt");
        core.log_error("restore_file", &e);
        let log = std::fs::read_to_string(core.data_dir().join("logs/lfvm.log")).unwrap();
        assert!(log.contains("FILE_IN_USE") && log.contains("docs/a.txt") && log.contains("restore_file"));
        assert!(!log.contains("机密正文"), "不记录说明文字或文件正文");
    }
}
