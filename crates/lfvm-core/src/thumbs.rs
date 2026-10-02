//! 图片缩略图（SRS 3.3.3.4 第 4 步）：PNG/JPEG 结果显示最长边 256 像素的缩略图，
//! 按内容摘要缓存在 `projects/<id>/thumbs/`；无法生成时给出原因，不影响其他结果。

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Condvar, Mutex};

use image::{ImageFormat, ImageReader, Limits};
use rusqlite::{OptionalExtension, params};

use crate::content::{IMAGE_MAX_BYTES, IMAGE_MAX_SIDE, probe_image, read_object};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::hash::is_valid_hash;
use crate::store::ObjectStore;
use crate::{Core, new_id};

pub const THUMB_MAX: u32 = 256;

/// 同时解码的图片数上限：一张 4000 万像素的图片解码后约 160 MB，限制并发以控制内存（LFVM-P-11）。
const DECODE_SLOTS: u32 = 2;

struct Slots {
    used: Mutex<u32>,
    cv: Condvar,
}

static SLOTS: Slots = Slots { used: Mutex::new(0), cv: Condvar::new() };

struct Permit;

impl Permit {
    fn acquire() -> Self {
        let mut used = SLOTS.used.lock().unwrap_or_else(|e| e.into_inner());
        while *used >= DECODE_SLOTS {
            used = SLOTS.cv.wait(used).unwrap_or_else(|e| e.into_inner());
        }
        *used += 1;
        Permit
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        *SLOTS.used.lock().unwrap_or_else(|e| e.into_inner()) -= 1;
        SLOTS.cv.notify_one();
    }
}

/// 生成缩略图（PNG 编码）。
pub fn make_thumbnail(bytes: &[u8]) -> Result<Vec<u8>, String> {
    probe_image(bytes)?;
    let _permit = Permit::acquire();
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|_| "图片无法读取".to_string())?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(IMAGE_MAX_SIDE);
    limits.max_image_height = Some(IMAGE_MAX_SIDE);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|_| "图片已损坏，无法生成缩略图".to_string())?;
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
            if e.code == ErrorCode::InvalidInput { CoreError::new(e.code, "图片超过 20 MB，不生成缩略图") } else { e }
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
