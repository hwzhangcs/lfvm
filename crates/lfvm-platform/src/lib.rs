//! 平台差异层。
//!
//! 业务核心（lfvm-core）只通过本 crate 访问与操作系统相关的行为，
//! 使同一套业务逻辑可在 Windows、macOS、Linux 上运行（SRS 7.3：首版在 Windows 验收）。

use std::fs::Metadata;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as imp;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

/// 目录项的分类。扫描时不跟随链接对象（SRS 2.3.1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    /// 符号链接、目录联接（junction）或其他重解析点。
    Link,
    /// 仅存于云端的占位文件（OneDrive、iCloud 等）。
    CloudPlaceholder,
    /// 设备文件、管道、套接字等。
    Other,
}

/// 根据 `symlink_metadata` 的结果给目录项分类。
pub fn classify(meta: &Metadata) -> EntryKind {
    imp::classify(meta)
}

/// 文件系统对路径大小写的处理方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CasePolicy {
    Sensitive,
    Insensitive,
}

impl CasePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            CasePolicy::Sensitive => "sensitive",
            CasePolicy::Insensitive => "insensitive",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "sensitive" => Some(CasePolicy::Sensitive),
            "insensitive" => Some(CasePolicy::Insensitive),
            _ => None,
        }
    }
}

/// 当前操作系统常见文件系统的默认大小写策略。
pub fn default_case_policy() -> CasePolicy {
    if cfg!(any(windows, target_os = "macos")) { CasePolicy::Insensitive } else { CasePolicy::Sensitive }
}

/// 探测目录所在文件系统是否区分大小写。
///
/// 加入项目时不得修改用户文件（SRS 3.3.1.1），因此不创建探测文件，
/// 而是取目录中一个含字母的已有条目，检查其大小写翻转后的名称是否指向同一对象。
/// 目录中没有可用条目时退回操作系统默认值。
pub fn probe_case_policy(dir: &Path) -> io::Result<CasePolicy> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let flipped: String = name
            .chars()
            .map(|c| {
                if c.is_lowercase() {
                    c.to_uppercase().next().unwrap_or(c)
                } else {
                    c.to_lowercase().next().unwrap_or(c)
                }
            })
            .collect();
        if flipped == name {
            continue;
        }
        let flipped_path = dir.join(&flipped);
        return Ok(match std::fs::symlink_metadata(&flipped_path) {
            Ok(m) => {
                let original = entry.metadata()?;
                if same_file_hint(&original, &m) {
                    CasePolicy::Insensitive
                } else {
                    // 大小写不同的两个条目同时存在，只可能是区分大小写的文件系统。
                    CasePolicy::Sensitive
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => CasePolicy::Sensitive,
            Err(e) => return Err(e),
        });
    }
    Ok(default_case_policy())
}

fn same_file_hint(a: &Metadata, b: &Metadata) -> bool {
    match (file_id(a), file_id(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a.len() == b.len() && a.modified().ok() == b.modified().ok(),
    }
}

/// 文件在文件系统中的唯一标识（Unix 为 设备号+inode）。用于扫描缓存判断“同一个文件”。
/// Windows 上暂不提供（std 的 file_index 仍不稳定），调用方需退回大小+修改时间。
pub fn file_id(meta: &Metadata) -> Option<(u64, u64)> {
    imp::file_id(meta)
}

/// 用已写好并校验过的临时文件替换目标文件。
///
/// 临时文件必须与目标位于同一目录（同一卷），这样替换是原子的：
/// 目标要么仍是旧内容，要么已是新内容，不会出现写了一半的文件。
pub fn atomic_replace(tmp: &Path, dst: &Path) -> io::Result<()> {
    // Windows 上刷盘（FlushFileBuffers）要求句柄有写权限，只读打开会报“拒绝访问”
    std::fs::OpenOptions::new().write(true).open(tmp)?.sync_all()?;
    imp::atomic_replace(tmp, dst)
}

/// 文件名在当前平台上不可用的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameIssue {
    Empty,
    /// 包含当前平台不允许的字符。
    ForbiddenChar(char),
    /// Windows 保留名，如 CON、NUL、COM1。
    ReservedName,
    /// Windows 不允许以空格或句点结尾。
    TrailingDotOrSpace,
    /// “.” 或 “..”。
    DotName,
}

/// 检查单个路径分量（文件名或目录名）能否在当前平台上创建。
/// 恢复、切换、展开、导出在写入前逐个检查，避免在其他系统上保存的版本写到一半才失败。
pub fn validate_component(name: &str) -> Result<(), NameIssue> {
    if name.is_empty() {
        return Err(NameIssue::Empty);
    }
    if name == "." || name == ".." {
        return Err(NameIssue::DotName);
    }
    if let Some(c) = name.chars().find(|c| *c == '/' || *c == '\0') {
        return Err(NameIssue::ForbiddenChar(c));
    }
    imp::validate_component(name)
}

/// 系统目录（SRS 3.3.1.1）：项目目录不得等于或位于其中。
/// 返回的路径与调用方比较时应先规范化。
pub fn protected_dirs() -> Vec<PathBuf> {
    imp::protected_dirs()
}

/// 路径是否为文件系统根（`/`、`C:\`、`\\server\share\`）。
pub fn is_filesystem_root(path: &Path) -> bool {
    path.parent().is_none()
}

/// 把目录元数据刷到磁盘，使其中的改名、新建在断电后仍然有效。Windows 上无需此操作。
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    imp::sync_dir(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_dot_names_and_slash() {
        assert_eq!(validate_component(""), Err(NameIssue::Empty));
        assert_eq!(validate_component(".."), Err(NameIssue::DotName));
        assert_eq!(validate_component("a/b"), Err(NameIssue::ForbiddenChar('/')));
        assert!(validate_component("报告.docx").is_ok());
    }

    #[test]
    fn classifies_symlink_as_link() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("t.txt");
        std::fs::write(&target, b"x").unwrap();
        assert_eq!(classify(&std::fs::symlink_metadata(&target).unwrap()), EntryKind::File);
        #[cfg(unix)]
        {
            let link = dir.path().join("l");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert_eq!(classify(&std::fs::symlink_metadata(&link).unwrap()), EntryKind::Link);
        }
    }

    #[test]
    fn atomic_replace_swaps_content() {
        let dir = tempfile::tempdir().unwrap();
        let dst = dir.path().join("a.txt");
        let tmp = dir.path().join(".lfvm~1.tmp");
        std::fs::write(&dst, b"old").unwrap();
        std::fs::write(&tmp, b"new").unwrap();
        atomic_replace(&tmp, &dst).unwrap();
        assert_eq!(std::fs::read(&dst).unwrap(), b"new");
        assert!(!tmp.exists());
    }

    #[test]
    fn probe_detects_policy_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Readme.md"), b"x").unwrap();
        let before: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        let policy = probe_case_policy(dir.path()).unwrap();
        let after: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(before.len(), after.len());
        if cfg!(target_os = "linux") {
            assert_eq!(policy, CasePolicy::Sensitive);
        }
    }

    #[test]
    fn root_detection() {
        assert!(is_filesystem_root(Path::new("/")));
        assert!(!is_filesystem_root(Path::new("/home")));
    }
}
