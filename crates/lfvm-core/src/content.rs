//! 历史文件内容的读取、识别与安全显示（SRS 3.3.2.2 内容比较范围、3.3.2.3 预览、LFVM-Q-07）。
//!
//! 历史文件只按纯文本或图片显示，任何内容都不会作为网页、脚本或程序执行。

use std::io::{Cursor, Read};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::hash::hash_bytes;
use crate::model::EntryType;
use crate::paths::{RelPath, path_key};
use crate::project::load_project;
use crate::store::ObjectStore;
use crate::Core;

/// 文本比较与预览上限（SRS 3.3.2.2）。
pub const TEXT_MAX_BYTES: u64 = 10 * 1024 * 1024;
pub const TEXT_MAX_LINES: usize = 100_000;
pub const TEXT_MAX_LINE_CHARS: usize = 100_000;
/// 图片比较与预览上限。
pub const IMAGE_MAX_BYTES: u64 = 20 * 1024 * 1024;
pub const IMAGE_MAX_PIXELS: u64 = 40_000_000;
pub const IMAGE_MAX_SIDE: u32 = 16_384;

/// 历史文件的来源：某个版本或某个安全备份。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceRef {
    Version { version_id: String },
    Backup { backup_id: String },
}

/// 来源中的一个文件。
#[derive(Debug, Clone)]
pub(crate) struct FileRecord {
    pub rel: String,
    pub entry_type: EntryType,
    pub size: Option<u64>,
    pub hash: Option<String>,
}

/// 确认来源属于该项目且内容可用。
pub(crate) fn check_source(conn: &Connection, project_id: &str, source: &SourceRef) -> CoreResult<()> {
    match source {
        SourceRef::Version { version_id } => {
            let state: Option<String> = conn
                .query_row(
                    "SELECT payload_state FROM versions WHERE version_id = ?1 AND project_id = ?2",
                    params![version_id, project_id],
                    |r| r.get(0),
                )
                .optional()?;
            match state.as_deref() {
                None => Err(CoreError::new(ErrorCode::NotFound, "找不到这个版本")),
                Some("cleared") => Err(cleared_error()),
                _ => Ok(()),
            }
        }
        SourceRef::Backup { backup_id } => {
            let status: Option<String> = conn
                .query_row(
                    "SELECT status FROM safety_backups WHERE backup_id = ?1 AND project_id = ?2",
                    params![backup_id, project_id],
                    |r| r.get(0),
                )
                .optional()?;
            match status.as_deref() {
                None => Err(CoreError::new(ErrorCode::NotFound, "找不到这个安全备份")),
                Some("ready") => Ok(()),
                Some("cleared") => Err(CoreError::new(ErrorCode::ContentCleared, "这个安全备份的内容已清理，无法找回")),
                Some(_) => Err(CoreError::new(ErrorCode::ContentCorrupted, "这个安全备份没有创建成功，不能作为恢复来源")),
            }
        }
    }
}

pub(crate) fn cleared_error() -> CoreError {
    CoreError::new(ErrorCode::ContentCleared, "这个版本的内容已清理，只保留了记录，无法浏览、比较或恢复")
}

/// 查找来源中的某个文件。
pub(crate) fn find_file(
    conn: &Connection,
    project_id: &str,
    source: &SourceRef,
    path: &str,
) -> CoreResult<FileRecord> {
    check_source(conn, project_id, source)?;
    let project = load_project(conn, project_id)?;
    let key = path_key(RelPath::parse(path)?.as_str(), project.policy);
    let (sql, id) = match source {
        SourceRef::Version { version_id } => (
            "SELECT relative_path, entry_type, size, content_hash FROM version_files WHERE version_id = ?1 AND path_key = ?2",
            version_id,
        ),
        SourceRef::Backup { backup_id } => (
            "SELECT relative_path, entry_type, size, content_hash FROM backup_files WHERE backup_id = ?1 AND path_key = ?2",
            backup_id,
        ),
    };
    conn.query_row(sql, params![id, key], |r| {
        Ok(FileRecord {
            rel: r.get(0)?,
            entry_type: EntryType::parse(&r.get::<_, String>(1)?),
            size: r.get::<_, Option<i64>>(2)?.map(|s| s as u64),
            hash: r.get(3)?,
        })
    })
    .optional()?
    .ok_or_else(|| CoreError::new(ErrorCode::NotFound, "历史中没有这个文件").with_path(path))
}

/// 读取内容对象（不超过 `limit` 字节），并核对摘要；不一致时报告“内容损坏”。
pub(crate) fn read_object(store: &ObjectStore, hash: &str, limit: u64) -> CoreResult<Vec<u8>> {
    let mut f = store.open(hash)?;
    let mut buf = Vec::new();
    (&mut f).take(limit + 1).read_to_end(&mut buf).map_err(CoreError::from)?;
    if buf.len() as u64 > limit {
        return Err(CoreError::new(ErrorCode::InvalidInput, "文件超出可显示的大小"));
    }
    if hash_bytes(&buf) != hash {
        return Err(CoreError::new(ErrorCode::ContentCorrupted, "历史内容已损坏（校验不一致）"));
    }
    Ok(buf)
}

// ───────────────────────── 识别与解码 ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    Png,
    Jpeg,
}

impl ImageFormat {
    pub fn mime(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
        }
    }
}

/// 按文件头判断是否为 PNG / JPEG（不看扩展名）。
pub fn sniff_image(head: &[u8]) -> Option<ImageFormat> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else {
        None
    }
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ImageInfo {
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
}

/// 检查图片是否在可显示范围内（单张 ≤20 MiB、像素 ≤4000 万、任一边 ≤16384）。
pub fn probe_image(bytes: &[u8]) -> Result<ImageInfo, String> {
    let format = sniff_image(bytes).ok_or("不是 PNG 或 JPEG 图片")?;
    if bytes.len() as u64 > IMAGE_MAX_BYTES {
        return Err("图片超过 20 MB，不显示内容".into());
    }
    let (width, height) = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "图片无法读取".to_string())?
        .into_dimensions()
        .map_err(|_| "图片已损坏或格式不受支持".to_string())?;
    if width > IMAGE_MAX_SIDE || height > IMAGE_MAX_SIDE {
        return Err(format!("图片边长 {width}×{height} 超过 16384 像素，不显示内容"));
    }
    if u64::from(width) * u64::from(height) > IMAGE_MAX_PIXELS {
        return Err(format!("图片像素数 {width}×{height} 超过 4000 万，不显示内容"));
    }
    Ok(ImageInfo { format, width, height })
}

/// 解码后的文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedText {
    pub text: String,
    pub encoding: &'static str,
}

/// 按 SRS 3.3.2.2 解码文本：UTF-8、带 BOM 的 UTF-8、带 BOM 的 UTF-16 LE/BE；
/// 超出大小/行数/行长，或含二进制内容时返回不能显示的原因。
pub fn decode_text(bytes: &[u8]) -> Result<DecodedText, String> {
    if bytes.len() as u64 > TEXT_MAX_BYTES {
        return Err("文本超过 10 MB，只显示是否有变化".into());
    }
    let (text, encoding) = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        (std::str::from_utf8(rest).map_err(|_| "文本编码无法识别")?.to_owned(), "UTF-8（带 BOM）")
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        (decode_utf16(rest, u16::from_le_bytes)?, "UTF-16 LE")
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        (decode_utf16(rest, u16::from_be_bytes)?, "UTF-16 BE")
    } else {
        (std::str::from_utf8(bytes).map_err(|_| "不是可识别的文本（可能是二进制文件，或编码不受支持）")?.to_owned(), "UTF-8")
    };
    if text.contains('\0') {
        return Err("含有二进制内容，只显示是否有变化".into());
    }
    let mut lines = 0usize;
    for line in text.split('\n') {
        lines += 1;
        if lines > TEXT_MAX_LINES {
            return Err("文本超过 100,000 行，只显示是否有变化".into());
        }
        if line.chars().count() > TEXT_MAX_LINE_CHARS {
            return Err("有一行超过 100,000 个字符，只显示是否有变化".into());
        }
    }
    Ok(DecodedText { text, encoding })
}

fn decode_utf16(bytes: &[u8], f: fn([u8; 2]) -> u16) -> Result<String, String> {
    if bytes.len() % 2 != 0 {
        return Err("文本编码无法识别".into());
    }
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| f([c[0], c[1]])).collect();
    String::from_utf16(&units).map_err(|_| "文本编码无法识别".into())
}

// ───────────────────────── 预览 ─────────────────────────

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreviewContent {
    Text { text: String, encoding: String },
    /// 图片经预览协议加载：`lfvm-preview://localhost/<project_id>/<hash>`
    Image { hash: String, info: ImageInfo },
    /// 不显示内容，给出原因。
    Unsupported { reason: String },
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct FilePreview {
    pub path: String,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub size: i64,
    pub hash: String,
    pub content: PreviewContent,
}

/// 读取一个内容对象并判断如何显示。
pub(crate) fn preview_object(store: &ObjectStore, hash: &str, size: u64) -> PreviewContent {
    let unsupported = |reason: String| PreviewContent::Unsupported { reason };
    let head = match store.open(hash) {
        Ok(mut f) => {
            let mut h = [0u8; 8];
            let n = f.read(&mut h).unwrap_or(0);
            h[..n].to_vec()
        }
        Err(e) => return unsupported(e.message),
    };
    if sniff_image(&head).is_some() {
        if size > IMAGE_MAX_BYTES {
            return unsupported("图片超过 20 MB，不显示内容".into());
        }
        return match read_object(store, hash, IMAGE_MAX_BYTES) {
            Ok(bytes) => match probe_image(&bytes) {
                Ok(info) => PreviewContent::Image { hash: hash.to_owned(), info },
                Err(r) => unsupported(r),
            },
            Err(e) => unsupported(e.message),
        };
    }
    if size > TEXT_MAX_BYTES {
        return unsupported("文件较大，且不是可显示的文本或图片".into());
    }
    match read_object(store, hash, TEXT_MAX_BYTES) {
        Ok(bytes) => match decode_text(&bytes) {
            Ok(d) => PreviewContent::Text { text: d.text, encoding: d.encoding.to_owned() },
            Err(r) => unsupported(r),
        },
        Err(e) => unsupported(e.message),
    }
}

impl Core {
    /// 预览历史文件（来自版本或安全备份）。不在磁盘上创建任何文件，也不改变工作区。
    pub fn preview_file(&self, project_id: &str, source: &SourceRef, path: &str) -> CoreResult<FilePreview> {
        let rec = find_file(&self.db(), project_id, source, path)?;
        let (Some(hash), EntryType::File) = (rec.hash.clone(), rec.entry_type) else {
            return Err(CoreError::new(ErrorCode::InvalidInput, "这是一个文件夹"));
        };
        let size = rec.size.unwrap_or(0);
        let store = ObjectStore::new(&self.project_store_dir(project_id));
        Ok(FilePreview { path: rec.rel, size: size as i64, content: preview_object(&store, &hash, size), hash })
    }

    /// 预览协议使用：读取图片对象的字节与类型。只提供本项目已登记的、可显示范围内的图片。
    pub fn image_object(&self, project_id: &str, hash: &str) -> CoreResult<(Vec<u8>, &'static str)> {
        let known: bool = self
            .db()
            .query_row(
                "SELECT 1 FROM content_objects WHERE project_id = ?1 AND content_hash = ?2",
                params![project_id, hash],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !known {
            return Err(CoreError::new(ErrorCode::NotFound, "找不到图片"));
        }
        let store = ObjectStore::new(&self.project_store_dir(project_id));
        let bytes = read_object(&store, hash, IMAGE_MAX_BYTES)?;
        let info = probe_image(&bytes).map_err(|r| CoreError::new(ErrorCode::InvalidInput, r))?;
        Ok((bytes, info.format.mime()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbImage::new(w, h)
            .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn decodes_supported_encodings() {
        assert_eq!(decode_text(b"abc\n").unwrap().encoding, "UTF-8");
        assert_eq!(decode_text(b"\xEF\xBB\xBFabc").unwrap().text, "abc");
        let le: Vec<u8> = [0xFF, 0xFE].into_iter().chain("中文".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert_eq!(decode_text(&le).unwrap().text, "中文");
        let be: Vec<u8> = [0xFE, 0xFF].into_iter().chain("中文".encode_utf16().flat_map(u16::to_be_bytes)).collect();
        assert_eq!(decode_text(&be).unwrap().text, "中文");
    }

    #[test]
    fn degrades_out_of_range_text() {
        // 无 BOM 的 UTF-16（LFVM-AT-11）
        let raw: Vec<u8> = "ab".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert!(decode_text(&raw).is_err());
        assert!(decode_text(&[0xFF, 0x00, 0x12]).is_err());
        assert!(decode_text(&"x".repeat(TEXT_MAX_LINE_CHARS + 1).into_bytes()).is_err());
        assert!(decode_text(&"\n".repeat(TEXT_MAX_LINES).into_bytes()).is_err());
        assert!(decode_text(&"x".repeat(TEXT_MAX_LINE_CHARS).into_bytes()).is_ok());
    }

    #[test]
    fn probes_images_with_limits() {
        let ok = probe_image(&png(4, 3)).unwrap();
        assert_eq!((ok.width, ok.height, ok.format), (4, 3, ImageFormat::Png));
        assert!(probe_image(b"<html><script>alert(1)</script>").is_err());
        let mut broken = png(4, 3);
        broken.truncate(12);
        assert!(probe_image(&broken).is_err());
        assert!(probe_image(&png(IMAGE_MAX_SIDE + 1, 1)).is_err());
    }

    #[test]
    fn html_is_previewed_as_plain_text() {
        let d = tempfile::tempdir().unwrap();
        let store = ObjectStore::new(d.path());
        store.ensure_dirs().unwrap();
        let html = b"<script>alert(1)</script>";
        let obj = store.ingest_bytes(html).unwrap();
        match preview_object(&store, &obj.hash, html.len() as u64) {
            PreviewContent::Text { text, .. } => assert_eq!(text.as_bytes(), html),
            other => panic!("应按纯文本显示：{other:?}"),
        }
    }

    #[test]
    fn corrupted_object_is_reported() {
        let d = tempfile::tempdir().unwrap();
        let store = ObjectStore::new(d.path());
        store.ensure_dirs().unwrap();
        let obj = store.ingest_bytes(b"hello").unwrap();
        std::fs::write(store.object_path(&obj.hash).unwrap(), b"HELLO").unwrap();
        assert_eq!(read_object(&store, &obj.hash, 100).unwrap_err().code, ErrorCode::ContentCorrupted);
    }
}
