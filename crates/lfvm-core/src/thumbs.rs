//! 图片缩略图（SRS 3.3.3.4 第 4 步）：PNG/JPEG 结果显示最长边 256 像素的缩略图，
//! 按内容摘要缓存在 `projects/<id>/thumbs/`；无法生成时给出原因，不影响其他结果。

use std::io::Cursor;
use std::path::PathBuf;

use image::ImageFormat;
use rusqlite::{OptionalExtension, params};

use crate::content::{IMAGE_MAX_BYTES, decode_image, read_object};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::hash::is_valid_hash;
use crate::store::ObjectStore;
use crate::{Core, new_id};

pub const THUMB_MAX: u32 = 256;

/// 生成缩略图（PNG 编码）。
pub fn make_thumbnail(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let (_, img) = decode_image(bytes)?;
    let thumb = img.thumbnail(THUMB_MAX, THUMB_MAX);
    let mut out = Vec::new();
    thumb.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).map_err(|_| "缩略图生成失败".to_string())?;
    Ok(out)
}

impl Core {
    fn thumb_path(&self, project_id: &str, hash: &str) -> PathBuf {
        self.project_store_dir(project_id).join("thumbs").join(&hash[..2]).join(format!("{hash}.png"))
    }

    /// 确保缩略图已生成。失败时返回原因（如超限、已损坏）。
    pub fn ensure_thumbnail(&self, project_id: &str, hash: &str) -> CoreResult<()> {
        if !is_valid_hash(hash) {
            return Err(CoreError::new(ErrorCode::InvalidInput, "无效的内容摘要"));
        }
        let path = self.thumb_path(project_id, hash);
        if path.is_file() {
            return Ok(());
        }
        let known = self
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
        let bytes = read_object(&store, hash, IMAGE_MAX_BYTES).map_err(|e| {
            if e.code == ErrorCode::InvalidInput {
                CoreError::new(e.code, "图片超过 20 MB，不生成缩略图")
            } else {
                e
            }
        })?;
        let png = make_thumbnail(&bytes).map_err(|r| CoreError::new(ErrorCode::InvalidInput, r))?;
        let dir = path.parent().expect("缩略图路径总有上级目录");
        std::fs::create_dir_all(dir).map_err(|e| CoreError::io(e, dir))?;
        let tmp = dir.join(format!("{}.part", new_id()));
        std::fs::write(&tmp, &png).map_err(|e| CoreError::io(e, &tmp))?;
        std::fs::rename(&tmp, &path).map_err(|e| CoreError::io(e, &path))?;
        Ok(())
    }

    /// 预览协议使用：缩略图字节（PNG）。
    pub fn thumbnail_bytes(&self, project_id: &str, hash: &str) -> CoreResult<Vec<u8>> {
        self.ensure_thumbnail(project_id, hash)?;
        let path = self.thumb_path(project_id, hash);
        std::fs::read(&path).map_err(|e| CoreError::io(e, &path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_keeps_aspect_ratio() {
        let mut png = Vec::new();
        image::RgbImage::new(1000, 500).write_to(&mut Cursor::new(&mut png), ImageFormat::Png).unwrap();
        let t = make_thumbnail(&png).unwrap();
        let img = image::load_from_memory(&t).unwrap();
        assert_eq!((img.width(), img.height()), (256, 128));
    }

    /// LFVM-AT-11：文件小但像素超过 4000 万的图片不生成缩略图。
    #[test]
    fn rejects_huge_or_broken_images() {
        // 7000×7000 = 4900 万像素，PNG 压缩后很小
        let mut png = Vec::new();
        image::GrayImage::new(7000, 7000).write_to(&mut Cursor::new(&mut png), ImageFormat::Png).unwrap();
        assert!(png.len() < 1024 * 1024);
        assert!(make_thumbnail(&png).unwrap_err().contains("4000 万"));
        assert!(make_thumbnail(b"\x89PNG\r\n\x1a\nbroken").is_err());
    }
}
