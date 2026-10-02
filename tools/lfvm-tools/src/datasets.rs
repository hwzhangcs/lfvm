//! 性能测试数据集（SRS 表 5-2），用固定随机种子生成，可重复。
//!
//! - D1：1000 个普通文件、合计 1 GiB，单文件不超过 100 MiB；含文本、图片、Office 文档、压缩包、
//!   零字节文件和 20 个空目录；
//! - D2：20 个版本 + 20 个安全备份，各 500 个文件记录，共 20,000 条（在 bench 中构建）；
//! - D3：文本与图片比较样本（上限以内、恰好等于上限、超过上限、损坏）；
//! - D4：500 个版本节点，默认历史与 9 个方案共享祖先，其中 10 个为内容已清理的占位（在 bench 中构建）。

use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;

/// xorshift64* 伪随机数：固定种子，结果可重复，不依赖外部库。
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }

    pub fn fill(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let v = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
    }
}

pub const SEED: u64 = 20_261_002;
const MIB: u64 = 1024 * 1024;

fn write_random(path: &Path, size: u64, rng: &mut Rng) -> std::io::Result<()> {
    let mut f = std::io::BufWriter::new(fs::File::create(path)?);
    let mut buf = vec![0u8; MIB as usize];
    let mut left = size;
    while left > 0 {
        let n = left.min(MIB) as usize;
        rng.fill(&mut buf[..n]);
        f.write_all(&buf[..n])?;
        left -= n as u64;
    }
    f.flush()
}

const WORDS: [&str; 16] = [
    "版本", "方案", "找回", "恢复", "报告", "图片", "课程", "设计", "lorem", "ipsum", "dolor", "sit", "amet", "data",
    "test", "文件",
];

pub fn text_of(size: u64, rng: &mut Rng) -> String {
    let mut s = String::with_capacity(size as usize + 64);
    while (s.len() as u64) < size {
        for _ in 0..(4 + rng.below(10)) {
            s.push_str(WORDS[rng.below(WORDS.len() as u64) as usize]);
            s.push(' ');
        }
        s.push('\n');
    }
    s
}

fn noise_image(w: u32, h: u32, rng: &mut Rng) -> image::RgbImage {
    let mut img = image::RgbImage::new(w, h);
    let mut buf = vec![0u8; (w * h * 3) as usize];
    rng.fill(&mut buf);
    img.copy_from_slice(&buf);
    img
}

/// 生成 D1。`scale` 按比例缩小总大小（1.0 为标准的 1 GiB），文件数固定为 1000。
/// 返回（文件数, 总字节数）。
pub fn gen_d1(dir: &Path, scale: f64) -> std::io::Result<(u32, u64)> {
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    fs::create_dir_all(dir)?;
    let mut rng = Rng::new(SEED);
    let total = ((1u64 << 30) as f64 * scale) as u64;
    // 8 个大文件占 62.5%，其余 992 个文件平分剩余部分
    let big = total * 5 / 8 / 8;
    let small_total = total - big * 8;
    let small = small_total / 992;
    let mut count = 0u32;
    let mut bytes = 0u64;
    for i in 0..20 {
        fs::create_dir_all(dir.join(format!("资料_{i:02}/子目录")))?;
        fs::create_dir_all(dir.join(format!("空目录_{i:02}")))?;
    }
    for i in 0..1000u32 {
        let sub = dir.join(format!("资料_{:02}", i % 20)).join(if i % 3 == 0 { "子目录" } else { "" });
        let (name, size): (String, u64) = if i < 8 {
            (format!("大文件_{i}.zip"), big.min(100 * MIB))
        } else if i < 18 {
            (format!("空文件_{i}.txt"), 0)
        } else {
            let ext = ["txt", "png", "jpg", "docx", "xlsx", "zip", "md", "pdf"][(i % 8) as usize];
            let jitter = rng.below(small / 2 + 1);
            (format!("文件_{i:04}.{ext}"), small / 2 + jitter + small / 4)
        };
        let path = sub.join(&name);
        match path.extension().and_then(|e| e.to_str()) {
            Some("txt" | "md") if size > 0 => fs::write(&path, text_of(size, &mut rng))?,
            Some("png") => {
                let side = ((size / 3) as f64).sqrt().max(8.0) as u32;
                noise_image(side, side, &mut rng).save(&path).map_err(std::io::Error::other)?;
            }
            Some("jpg") => {
                let side = ((size / 2) as f64).sqrt().max(8.0) as u32;
                let mut out = Vec::new();
                noise_image(side, side, &mut rng)
                    .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Jpeg)
                    .map_err(std::io::Error::other)?;
                fs::write(&path, out)?;
            }
            _ => write_random(&path, size, &mut rng)?,
        }
        bytes += fs::metadata(&path)?.len();
        count += 1;
    }
    Ok((count, bytes))
}

/// D3 的一对样本：A、B 两个版本中的同名文件。
pub struct Pair {
    pub name: &'static str,
    pub a: Vec<u8>,
    pub b: Vec<u8>,
}

fn png_bytes(img: &image::DynamicImage) -> Vec<u8> {
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).expect("编码 PNG");
    out
}

fn jpeg_bytes(img: &image::DynamicImage) -> Vec<u8> {
    let mut out = Vec::new();
    img.to_rgb8().write_to(&mut Cursor::new(&mut out), image::ImageFormat::Jpeg).expect("编码 JPEG");
    out
}

/// 生成 D3 样本对。
pub fn d3_pairs() -> Vec<Pair> {
    let mut rng = Rng::new(SEED + 3);
    let base = text_of(5 * MIB, &mut rng);
    let mut few: Vec<String> = base.lines().map(str::to_owned).collect();
    for i in 0..10 {
        let at = (i * 997) % few.len();
        few[at] = format!("修改的第 {i} 行");
    }
    let few = few.join("\n") + "\n";
    let other = text_of(5 * MIB, &mut rng);
    let long_line = "长".repeat(100_001);

    let photo = image::DynamicImage::ImageRgb8(noise_image(1600, 1200, &mut rng));
    let photo2 = image::DynamicImage::ImageRgb8(noise_image(1600, 1200, &mut rng));
    // 恰好 4000 万像素（8000×5000），纯色以便压缩后远小于 20 MB
    let at_limit = image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(8000, 5000, image::Luma([200])));
    let over_limit = image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(8001, 5000, image::Luma([200])));
    let mut broken = png_bytes(&photo);
    broken.truncate(broken.len() / 3);
    let mut broken_jpg = jpeg_bytes(&photo);
    broken_jpg.truncate(broken_jpg.len() / 3);

    vec![
        Pair { name: "文本_无变化.txt", a: base.clone().into_bytes(), b: base.clone().into_bytes() },
        Pair { name: "文本_改10行.txt", a: base.clone().into_bytes(), b: few.into_bytes() },
        Pair { name: "文本_完全不同.txt", a: base.into_bytes(), b: other.into_bytes() },
        Pair { name: "文本_超长行.txt", a: b"short\n".to_vec(), b: long_line.into_bytes() },
        Pair { name: "图片_上限内.png", a: png_bytes(&photo), b: png_bytes(&photo2) },
        Pair { name: "图片_上限内.jpg", a: jpeg_bytes(&photo), b: jpeg_bytes(&photo2) },
        Pair { name: "图片_恰好上限.png", a: png_bytes(&at_limit), b: png_bytes(&at_limit.brighten(10)) },
        Pair { name: "图片_恰好上限.jpg", a: jpeg_bytes(&at_limit), b: jpeg_bytes(&at_limit.brighten(10)) },
        Pair { name: "图片_超过上限.png", a: png_bytes(&over_limit), b: png_bytes(&over_limit.brighten(10)) },
        Pair { name: "图片_超过上限.jpg", a: jpeg_bytes(&over_limit), b: jpeg_bytes(&over_limit.brighten(10)) },
        Pair { name: "图片_损坏.png", a: png_bytes(&photo), b: broken },
        Pair { name: "图片_损坏.jpg", a: jpeg_bytes(&photo), b: broken_jpg },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        assert_eq!((0..5).map(|_| a.next_u64()).collect::<Vec<_>>(), (0..5).map(|_| b.next_u64()).collect::<Vec<_>>());
    }

    #[test]
    fn small_d1_has_expected_shape() {
        let d = std::env::temp_dir().join(format!("lfvm-d1-test-{}", std::process::id()));
        let (n, bytes) = gen_d1(&d, 0.002).unwrap();
        assert_eq!(n, 1000);
        assert!(bytes > 1_000_000);
        let empty = fs::read_dir(&d)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("空目录"))
            .count();
        assert_eq!(empty, 20);
        fs::remove_dir_all(&d).unwrap();
    }
}
