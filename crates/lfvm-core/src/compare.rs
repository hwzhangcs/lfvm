//! 比较两个版本（SRS 3.3.2.2）。
//!
//! - 文件级：按相对路径列出 A→B 的新增、删除、内容修改和“文件↔文件夹”类型变化；
//!   内容是否修改以 SHA-256 判断，仅修改时间变化不算修改；B 保存时被排除的路径标为“因排除未纳入”；
//! - 内容级：文本按行比较（换行符差异单独提示），PNG/JPEG 并排显示，其他类型只显示状态。

use std::time::{Duration, Instant};

use lfvm_platform::CasePolicy;
use serde::Serialize;
use similar::{Algorithm, ChangeTag, TextDiff};

use crate::Core;
use crate::changes::{Manifest, load_manifest};
use crate::content::{
    IMAGE_MAX_BYTES, ImageInfo, SourceRef, TEXT_MAX_BYTES, check_source, decode_text, probe_image, read_object,
    sniff_image,
};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::exclude::{self, Matcher};
use crate::model::EntryType;
use crate::paths::{RelPath, path_key};
use crate::project::{VersionBrief, load_project, version_brief};
use crate::store::ObjectStore;

/// 文本比较的时间上限，超过即降级为文件级状态。
pub const DIFF_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum CompareKind {
    Added,
    Deleted,
    Modified,
    TypeChanged,
    /// A 中有、B 中没有，且该路径在 B 保存时被排除（不计为删除）。
    ExcludedInB,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct CompareItem {
    pub path: String,
    pub kind: CompareKind,
    /// B 中的类型；B 中没有时为 A 中的类型。
    pub entry_type: EntryType,
    #[cfg_attr(feature = "specta", specta(type = Option<specta_typescript::Number>))]
    pub size_a: Option<i64>,
    #[cfg_attr(feature = "specta", specta(type = Option<specta_typescript::Number>))]
    pub size_b: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct CompareCounts {
    pub added: u32,
    pub deleted: u32,
    pub modified: u32,
    pub type_changed: u32,
    pub excluded_in_b: u32,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct CompareResult {
    pub a: VersionBrief,
    pub b: VersionBrief,
    /// 只比较了这个文件夹（null 表示整个项目）。
    pub scope: Option<String>,
    pub items: Vec<CompareItem>,
    pub counts: CompareCounts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum LineTag {
    Equal,
    Insert,
    Delete,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DiffLine {
    pub tag: LineTag,
    /// A 中的行号（从 1 开始）。
    pub old_no: Option<u32>,
    /// B 中的行号（从 1 开始）。
    pub new_no: Option<u32>,
    pub text: String,
}

/// 图片比较中的一侧。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ImageSide {
    pub hash: String,
    /// 可显示时给出尺寸；不可显示时为 null，并给出原因。
    pub info: Option<ImageInfo>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileDiff {
    Text {
        lines: Vec<DiffLine>,
        encoding_a: Option<String>,
        encoding_b: Option<String>,
        /// 换行符或文件末尾换行的差异说明。
        notes: Vec<String>,
    },
    /// 一侧不存在时为 null。
    Image { a: Option<ImageSide>, b: Option<ImageSide> },
    /// 只显示文件级状态，并说明原因。
    Unsupported { reason: String },
}

impl Core {
    /// 比较版本 A 与 B（方向 A→B），可只比较某个文件夹。
    pub fn compare_versions(
        &self,
        project_id: &str,
        a: &str,
        b: &str,
        scope: Option<&str>,
    ) -> CoreResult<CompareResult> {
        let db = self.db();
        let project = load_project(&db, project_id)?;
        for v in [a, b] {
            check_source(&db, project_id, &SourceRef::Version { version_id: v.to_owned() })?;
        }
        let scope = scope.map(RelPath::parse).transpose()?;
        let ma = load_manifest(&db, a)?;
        let mb = load_manifest(&db, b)?;
        let b_rules: String =
            db.query_row("SELECT exclusion_rules_snapshot FROM versions WHERE version_id = ?1", [b], |r| r.get(0))?;
        let b_matcher = Matcher::new(&exclude::parse_snapshot(&b_rules), project.policy)?;
        let (items, counts) = diff_manifests(&ma, &mb, &b_matcher, scope.as_ref(), project.policy);
        Ok(CompareResult {
            a: version_brief(&db, a)?,
            b: version_brief(&db, b)?,
            scope: scope.map(|s| s.to_string()),
            items,
            counts,
        })
    }

    /// 比较某个文件在两个版本中的内容。
    pub fn diff_file(&self, project_id: &str, a: &str, b: &str, path: &str) -> CoreResult<FileDiff> {
        let _leases = (self.lease(a), self.lease(b));
        let (fa, fb) = {
            let db = self.db();
            let project = load_project(&db, project_id)?;
            let key = path_key(RelPath::parse(path)?.as_str(), project.policy);
            let mut sides = Vec::new();
            for v in [a, b] {
                check_source(&db, project_id, &SourceRef::Version { version_id: v.to_owned() })?;
                let m = load_manifest(&db, v)?;
                sides.push(m.get(&key).filter(|e| e.entry_type == EntryType::File).and_then(|e| e.hash.clone()));
            }
            (sides[0].clone(), sides[1].clone())
        };
        if fa.is_none() && fb.is_none() {
            return Err(CoreError::new(ErrorCode::NotFound, "两个版本中都没有这个文件").with_path(path));
        }
        let store = ObjectStore::new(&self.project_store_dir(project_id));
        Ok(diff_objects(&store, fa.as_deref(), fb.as_deref()))
    }
}

pub(crate) fn diff_manifests(
    a: &Manifest,
    b: &Manifest,
    b_matcher: &Matcher,
    scope: Option<&RelPath>,
    policy: CasePolicy,
) -> (Vec<CompareItem>, CompareCounts) {
    let in_scope =
        |rel: &str| scope.is_none_or(|s| RelPath::parse(rel).is_ok_and(|r| r.is_within(s, policy) && r != *s));
    let size = |s: Option<u64>| s.map(|v| v as i64);
    let mut items = Vec::new();
    let mut c = CompareCounts::default();
    for (key, eb) in b {
        if !in_scope(&eb.rel) {
            continue;
        }
        let kind = match a.get(key) {
            None => CompareKind::Added,
            Some(ea) if ea.entry_type != eb.entry_type => CompareKind::TypeChanged,
            Some(ea) if eb.entry_type == EntryType::File && ea.hash != eb.hash => CompareKind::Modified,
            Some(_) => continue,
        };
        let size_a = a.get(key).and_then(|e| size(e.size));
        items.push(CompareItem {
            path: eb.rel.clone(),
            kind,
            entry_type: eb.entry_type,
            size_a,
            size_b: size(eb.size),
        });
    }
    for (key, ea) in a {
        if b.contains_key(key) || !in_scope(&ea.rel) {
            continue;
        }
        let excluded = RelPath::parse(&ea.rel).is_ok_and(|r| b_matcher.is_excluded(&r, ea.entry_type.is_dir()));
        items.push(CompareItem {
            path: ea.rel.clone(),
            kind: if excluded { CompareKind::ExcludedInB } else { CompareKind::Deleted },
            entry_type: ea.entry_type,
            size_a: size(ea.size),
            size_b: None,
        });
    }
    for i in &items {
        match i.kind {
            CompareKind::Added => c.added += 1,
            CompareKind::Deleted => c.deleted += 1,
            CompareKind::Modified => c.modified += 1,
            CompareKind::TypeChanged => c.type_changed += 1,
            CompareKind::ExcludedInB => c.excluded_in_b += 1,
        }
    }
    items.sort_by(|x, y| x.path.cmp(&y.path));
    (items, c)
}

fn head_of(store: &ObjectStore, hash: &str) -> Vec<u8> {
    use std::io::Read;
    let mut h = vec![0u8; 8];
    let n = store.open(hash).ok().and_then(|mut f| f.read(&mut h).ok()).unwrap_or(0);
    h.truncate(n);
    h
}

pub(crate) fn diff_objects(store: &ObjectStore, a: Option<&str>, b: Option<&str>) -> FileDiff {
    let is_image = [a, b].iter().flatten().any(|h| sniff_image(&head_of(store, h)).is_some());
    if is_image {
        let side = |h: Option<&str>| {
            h.map(|h| {
                match read_object(store, h, IMAGE_MAX_BYTES).map_err(|e| e.message).and_then(|b| probe_image(&b)) {
                    Ok(info) => ImageSide { hash: h.to_owned(), info: Some(info), reason: None },
                    Err(r) => ImageSide { hash: h.to_owned(), info: None, reason: Some(r) },
                }
            })
        };
        return FileDiff::Image { a: side(a), b: side(b) };
    }
    let load = |h: Option<&str>| -> Result<Option<crate::content::DecodedText>, String> {
        match h {
            None => Ok(None),
            Some(h) => {
                let bytes = read_object(store, h, TEXT_MAX_BYTES).map_err(|e| {
                    if e.code == ErrorCode::InvalidInput {
                        "文本超过 10 MB，只显示是否有变化".to_owned()
                    } else {
                        e.message
                    }
                })?;
                decode_text(&bytes).map(Some)
            }
        }
    };
    let (ta, tb) = match (load(a), load(b)) {
        (Ok(x), Ok(y)) => (x, y),
        (Err(r), _) | (_, Err(r)) => return FileDiff::Unsupported { reason: r },
    };
    let old = ta.as_ref().map_or("", |t| t.text.as_str());
    let new = tb.as_ref().map_or("", |t| t.text.as_str());
    match diff_text(old, new) {
        Ok(lines) => FileDiff::Text {
            lines,
            encoding_a: ta.as_ref().map(|t| t.encoding.to_owned()),
            encoding_b: tb.as_ref().map(|t| t.encoding.to_owned()),
            notes: if ta.is_some() && tb.is_some() { eol_notes(old, new) } else { Vec::new() },
        },
        Err(r) => FileDiff::Unsupported { reason: r },
    }
}

/// 按行比较。行尾的 “\r” 不参与比较，换行符差异由 [`eol_notes`] 单独提示。
pub fn diff_text(old: &str, new: &str) -> Result<Vec<DiffLine>, String> {
    let ol: Vec<&str> = old.lines().collect();
    let nl: Vec<&str> = new.lines().collect();
    let started = Instant::now();
    let diff = TextDiff::configure().algorithm(Algorithm::Myers).timeout(DIFF_TIMEOUT).diff_slices(&ol, &nl);
    if started.elapsed() >= DIFF_TIMEOUT {
        return Err("比较耗时超过 5 秒，只显示是否有变化".into());
    }
    let mut out = Vec::with_capacity(ol.len().max(nl.len()));
    for op in diff.ops() {
        for ch in diff.iter_changes(op) {
            out.push(DiffLine {
                tag: match ch.tag() {
                    ChangeTag::Equal => LineTag::Equal,
                    ChangeTag::Insert => LineTag::Insert,
                    ChangeTag::Delete => LineTag::Delete,
                },
                old_no: ch.old_index().map(|i| i as u32 + 1),
                new_no: ch.new_index().map(|i| i as u32 + 1),
                text: ch.value().to_owned(),
            });
        }
    }
    Ok(out)
}

fn eol_style(s: &str) -> Option<&'static str> {
    let crlf = s.matches("\r\n").count();
    let lf = s.matches('\n').count() - crlf;
    match (crlf, lf) {
        (0, 0) => None,
        (_, 0) => Some("CRLF（Windows）"),
        (0, _) => Some("LF（Unix）"),
        _ => Some("混合"),
    }
}

/// 换行符或文件末尾换行的差异（SRS 3.3.2.2 第 3 步）。
pub fn eol_notes(old: &str, new: &str) -> Vec<String> {
    let mut notes = Vec::new();
    if let (Some(a), Some(b)) = (eol_style(old), eol_style(new))
        && a != b
    {
        notes.push(format!("换行符不同：A 为 {a}，B 为 {b}"));
    }
    let (ea, eb) = (old.ends_with('\n'), new.ends_with('\n'));
    if !old.is_empty() && !new.is_empty() && ea != eb {
        notes.push(if eb {
            "B 在文件末尾增加了换行".into()
        } else {
            "B 去掉了文件末尾的换行".into()
        });
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::ManifestEntry;
    use crate::exclude::{ExclusionRule, RuleType};

    const S: CasePolicy = CasePolicy::Sensitive;

    fn m(entries: &[(&str, EntryType, Option<&str>)]) -> Manifest {
        entries
            .iter()
            .map(|(p, t, h)| {
                (
                    (*p).to_owned(),
                    ManifestEntry {
                        rel: (*p).to_owned(),
                        entry_type: *t,
                        size: h.map(|_| 1),
                        hash: h.map(str::to_owned),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn manifest_diff_is_directional() {
        use EntryType::*;
        let a = m(&[
            ("same", File, Some("1")),
            ("mod", File, Some("1")),
            ("gone", File, Some("1")),
            ("t", Directory, None),
        ]);
        let b =
            m(&[("same", File, Some("1")), ("mod", File, Some("2")), ("new", File, Some("1")), ("t", File, Some("1"))]);
        let (items, c) = diff_manifests(&a, &b, &Matcher::empty(S), None, S);
        let got: Vec<_> = items.iter().map(|i| (i.path.as_str(), i.kind)).collect();
        assert_eq!(
            got,
            [
                ("gone", CompareKind::Deleted),
                ("mod", CompareKind::Modified),
                ("new", CompareKind::Added),
                ("t", CompareKind::TypeChanged)
            ]
        );
        assert_eq!((c.added, c.deleted, c.modified, c.type_changed), (1, 1, 1, 1));
        // 交换 A、B 后新增与删除互换（AC-0010）
        let (rev, _) = diff_manifests(&b, &a, &Matcher::empty(S), None, S);
        assert_eq!(rev.iter().find(|i| i.path == "gone").unwrap().kind, CompareKind::Added);
        assert_eq!(rev.iter().find(|i| i.path == "new").unwrap().kind, CompareKind::Deleted);
        // 同一版本没有差异
        assert!(diff_manifests(&a, &a, &Matcher::empty(S), None, S).0.is_empty());
    }

    #[test]
    fn excluded_in_b_and_scope() {
        use EntryType::*;
        let a = m(&[
            ("cache", Directory, None),
            ("cache/x", File, Some("1")),
            ("src/a", File, Some("1")),
            ("doc/b", File, Some("1")),
        ]);
        let b = m(&[]);
        let rule = ExclusionRule {
            relative_path: "cache".into(),
            entry_type: RuleType::Directory,
            is_system_default: false,
            enabled: true,
        };
        let bm = Matcher::new(&[rule], S).unwrap();
        let (items, c) = diff_manifests(&a, &b, &bm, None, S);
        assert_eq!(c.excluded_in_b, 2);
        assert_eq!(c.deleted, 2);
        assert!(items.iter().any(|i| i.path == "cache/x" && i.kind == CompareKind::ExcludedInB));
        let scope = RelPath::parse("src").unwrap();
        let (items, _) = diff_manifests(&a, &b, &bm, Some(&scope), S);
        assert_eq!(items.iter().map(|i| i.path.as_str()).collect::<Vec<_>>(), ["src/a"]);
    }

    #[test]
    fn text_diff_lines_and_eol_notes() {
        let lines = diff_text("a\nb\nc\n", "a\nB\nc\nd\n").unwrap();
        let tags: Vec<_> = lines.iter().map(|l| (l.tag, l.text.as_str())).collect();
        assert_eq!(
            tags,
            [
                (LineTag::Equal, "a"),
                (LineTag::Delete, "b"),
                (LineTag::Insert, "B"),
                (LineTag::Equal, "c"),
                (LineTag::Insert, "d")
            ]
        );
        assert_eq!(lines[4].new_no, Some(4));
        // 只有换行符不同：行内容全部相同，另行提示
        let crlf = diff_text("a\r\nb\r\n", "a\nb\n").unwrap();
        assert!(crlf.iter().all(|l| l.tag == LineTag::Equal));
        assert_eq!(eol_notes("a\r\nb\r\n", "a\nb\n").len(), 1);
        assert_eq!(eol_notes("a\nb", "a\nb\n"), ["B 在文件末尾增加了换行"]);
        assert!(eol_notes("a\n", "a\n").is_empty());
    }

    #[test]
    fn diff_objects_text_image_and_binary() {
        let d = tempfile::tempdir().unwrap();
        let store = ObjectStore::new(d.path());
        store.ensure_dirs().unwrap();
        let t1 = store.ingest_bytes(b"x\n").unwrap().hash;
        let t2 = store.ingest_bytes(b"y\n").unwrap().hash;
        assert!(matches!(diff_objects(&store, Some(&t1), Some(&t2)), FileDiff::Text { .. }));
        // 新增文件：与空内容比较
        match diff_objects(&store, None, Some(&t2)) {
            FileDiff::Text { lines, .. } => assert_eq!(lines[0].tag, LineTag::Insert),
            other => panic!("{other:?}"),
        }
        let mut png = Vec::new();
        image::RgbImage::new(2, 2).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let img = store.ingest_bytes(&png).unwrap().hash;
        match diff_objects(&store, Some(&img), None) {
            FileDiff::Image { a: Some(a), b: None } => assert_eq!(a.info.unwrap().width, 2),
            other => panic!("{other:?}"),
        }
        let bin = store.ingest_bytes(&[0u8, 1, 2, 0xFF]).unwrap().hash;
        assert!(matches!(diff_objects(&store, Some(&bin), Some(&t1)), FileDiff::Unsupported { .. }));
    }
}
