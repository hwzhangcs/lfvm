//! 路径规范化与范围校验（SRS 2.3.1、3.3.1.2、LFVM-Q-07）。
//!
//! 版本清单只保存项目内相对路径（LFVM-O-03），统一用 “/” 分隔并转为 Unicode NFC，
//! 这样同一份历史在 Windows、macOS（文件名常为 NFD）、Linux 上得到相同的路径。

use std::fmt;
use std::path::{Component, Path, PathBuf};

use lfvm_platform::CasePolicy;
use unicode_normalization::UnicodeNormalization;

use crate::error::{CoreError, CoreResult, ErrorCode};

/// 项目内相对路径：非空、无 “.”/“..”、以 “/” 分隔、NFC。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RelPath(String);

impl RelPath {
    /// 解析用户输入或数据库中的相对路径。接受 “\” 作为分隔符。
    pub fn parse(input: &str) -> CoreResult<Self> {
        let invalid = |why: &str| {
            CoreError::new(ErrorCode::InvalidPath, format!("路径“{input}”无效：{why}"))
        };
        let s = input.replace('\\', "/");
        if s.starts_with('/') || has_drive_prefix(&s) {
            return Err(invalid("不能使用绝对路径"));
        }
        let mut parts = Vec::new();
        for part in s.split('/') {
            match part {
                "" => continue,
                "." | ".." => return Err(invalid("不能包含“.”或“..”")),
                p if p.chars().any(char::is_control) => return Err(invalid("包含控制字符")),
                p => parts.push(p.nfc().collect::<String>()),
            }
        }
        if parts.is_empty() {
            return Err(invalid("路径为空"));
        }
        Ok(Self(parts.join("/")))
    }

    /// 由父路径和单个名称构造（扫描时使用）。名称中含 “/” 或 “\” 时拒绝，
    /// 以免在其他系统上被拆成多级目录。
    pub fn child(parent: Option<&RelPath>, name: &str) -> CoreResult<Self> {
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains(['/', '\\'])
            || name.chars().any(char::is_control)
        {
            return Err(CoreError::new(
                ErrorCode::InvalidPath,
                format!("名称“{name}”含有无法处理的字符"),
            ));
        }
        let name: String = name.nfc().collect();
        Ok(match parent {
            Some(p) => RelPath(format!("{}/{}", p.0, name)),
            None => RelPath(name),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }

    /// 用于比较、去重和匹配的键：大小写不敏感的文件系统上转为小写。
    pub fn key(&self, policy: CasePolicy) -> String {
        path_key(&self.0, policy)
    }

    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }

    /// 扩展名（不含点）。“.gitignore” 这类以点开头的名称视为没有扩展名。
    pub fn extension(&self) -> Option<&str> {
        let name = self.file_name();
        match name.rfind('.') {
            Some(0) | None => None,
            Some(i) => Some(&name[i + 1..]),
        }
    }

    pub fn parent(&self) -> Option<RelPath> {
        self.0.rfind('/').map(|i| RelPath(self.0[..i].to_owned()))
    }

    pub fn join(&self, name: &str) -> CoreResult<RelPath> {
        RelPath::parse(&format!("{}/{}", self.0, name))
    }

    /// 是否等于 `dir` 或位于 `dir` 之下。按目录边界匹配：“a/b” 不匹配 “a/b2”。
    pub fn is_within(&self, dir: &RelPath, policy: CasePolicy) -> bool {
        let me = self.key(policy);
        let d = dir.key(policy);
        me == d || (me.len() > d.len() && me.starts_with(&d) && me.as_bytes()[d.len()] == b'/')
    }

    /// 在项目根目录下的实际路径。
    pub fn to_path(&self, root: &Path) -> PathBuf {
        let mut p = root.to_path_buf();
        p.extend(self.components());
        p
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn has_drive_prefix(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// 字符串路径的比较键。
pub fn path_key(s: &str, policy: CasePolicy) -> String {
    let nfc: String = s.nfc().collect();
    match policy {
        CasePolicy::Sensitive => nfc,
        CasePolicy::Insensitive => nfc.to_lowercase(),
    }
}

/// 求 `abs` 相对 `root` 的路径。`abs` 必须位于 `root` 之内。
pub fn relative_to(root: &Path, abs: &Path) -> CoreResult<RelPath> {
    let rest = abs.strip_prefix(root).map_err(|_| {
        CoreError::new(ErrorCode::PathOutOfScope, "路径不在项目文件夹内").with_path(abs)
    })?;
    let mut parts = Vec::new();
    for c in rest.components() {
        match c {
            Component::Normal(os) => parts.push(os.to_str().ok_or_else(|| {
                CoreError::new(ErrorCode::InvalidPath, "路径中含有无法处理的字符").with_path(abs)
            })?),
            _ => {
                return Err(
                    CoreError::new(ErrorCode::InvalidPath, "路径无法解析").with_path(abs)
                );
            }
        }
    }
    RelPath::parse(&parts.join("/"))
}

/// 解析为真实的绝对路径（解开链接与 “..”），Windows 上去掉 `\\?\` 前缀。
pub fn canonical(path: &Path) -> CoreResult<PathBuf> {
    dunce::canonicalize(path).map_err(|e| CoreError::io(e, path))
}

/// 两个绝对路径是否相同或一方包含另一方。调用前应先 [`canonical`]。
pub fn same_or_nested(a: &Path, b: &Path, policy: CasePolicy) -> bool {
    let ka = component_keys(a, policy);
    let kb = component_keys(b, policy);
    let n = ka.len().min(kb.len());
    ka[..n] == kb[..n]
}

/// `inner` 是否等于 `outer` 或位于其中。
pub fn is_inside(inner: &Path, outer: &Path, policy: CasePolicy) -> bool {
    let ki = component_keys(inner, policy);
    let ko = component_keys(outer, policy);
    ki.len() >= ko.len() && ki[..ko.len()] == ko[..]
}

fn component_keys(p: &Path, policy: CasePolicy) -> Vec<String> {
    p.components()
        .filter(|c| !matches!(c, Component::CurDir))
        .map(|c| path_key(&c.as_os_str().to_string_lossy(), policy))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: CasePolicy = CasePolicy::Sensitive;
    const I: CasePolicy = CasePolicy::Insensitive;

    #[test]
    fn parse_normalizes_separators() {
        assert_eq!(RelPath::parse("a\\b//c/").unwrap().as_str(), "a/b/c");
    }

    #[test]
    fn parse_rejects_escape_and_absolute() {
        for bad in ["", "/", "../x", "a/../b", "./a", "/etc/passwd", "C:/x", "c:x", "a\u{0}b"] {
            assert!(RelPath::parse(bad).is_err(), "{bad:?} 应被拒绝");
        }
    }

    #[test]
    fn nfd_and_nfc_have_same_key() {
        let nfc = RelPath::parse("caf\u{e9}.txt").unwrap();
        let nfd = RelPath::parse("cafe\u{301}.txt").unwrap();
        assert_eq!(nfc, nfd);
    }

    #[test]
    fn is_within_respects_directory_boundary() {
        let ab = RelPath::parse("a/b").unwrap();
        assert!(RelPath::parse("a/b").unwrap().is_within(&ab, S));
        assert!(RelPath::parse("a/b/c.txt").unwrap().is_within(&ab, S));
        assert!(!RelPath::parse("a/b2").unwrap().is_within(&ab, S));
        assert!(!RelPath::parse("A/B/c").unwrap().is_within(&ab, S));
        assert!(RelPath::parse("A/B/c").unwrap().is_within(&ab, I));
    }

    #[test]
    fn name_and_extension() {
        let p = RelPath::parse("docs/报告.final.DOCX").unwrap();
        assert_eq!(p.file_name(), "报告.final.DOCX");
        assert_eq!(p.extension(), Some("DOCX"));
        assert_eq!(RelPath::parse(".gitignore").unwrap().extension(), None);
        assert_eq!(p.parent().unwrap().as_str(), "docs");
        assert_eq!(RelPath::parse("x").unwrap().parent(), None);
    }

    #[test]
    fn relative_to_root() {
        let root = Path::new("/w/proj");
        assert_eq!(relative_to(root, Path::new("/w/proj/a/b.txt")).unwrap().as_str(), "a/b.txt");
        assert!(relative_to(root, Path::new("/w/other/a")).is_err());
    }

    #[test]
    fn nesting_checks() {
        let a = Path::new("/w/proj");
        assert!(same_or_nested(a, Path::new("/w/proj/sub"), S));
        assert!(same_or_nested(Path::new("/w/proj/sub"), a, S));
        assert!(same_or_nested(a, a, S));
        assert!(!same_or_nested(a, Path::new("/w/proj2"), S));
        assert!(same_or_nested(a, Path::new("/W/Proj/x"), I));
        assert!(is_inside(Path::new("/w/proj/x"), a, S));
        assert!(!is_inside(a, Path::new("/w/proj/x"), S));
    }
}
