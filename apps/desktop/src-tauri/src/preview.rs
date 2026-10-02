//! 图片预览协议 `lfvm-preview`（LFVM-Q-07）。
//!
//! 地址形如 `lfvm-preview://localhost/<project_id>.<hash>`（Windows 上为 `http://lfvm-preview.localhost/…`），
//! 只提供本项目已登记、在可显示范围内的 PNG/JPEG 内容对象，响应类型固定为图片且禁止浏览器猜测类型，
//! 因此历史文件不会被当作网页或脚本执行。

use tauri::http::{Request, Response, StatusCode, header};
use tauri::{Manager, UriSchemeContext, UriSchemeResponder, Wry};

use crate::AppState;

pub const SCHEME: &str = "lfvm-preview";

pub fn handle(ctx: UriSchemeContext<'_, Wry>, request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let core = ctx.app_handle().state::<AppState>().core.clone();
    let target = request.uri().path().trim_start_matches('/').to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let found = target
            .split_once('.')
            .filter(|(p, h)| p.chars().all(|c| c.is_ascii_alphanumeric()) && lfvm_core::hash::is_valid_hash(h))
            .and_then(|(project, hash)| core.image_object(project, hash).ok());
        let response = match found {
            Some((bytes, mime)) => Response::builder()
                .header(header::CONTENT_TYPE, mime)
                .header("X-Content-Type-Options", "nosniff")
                .header(header::CACHE_CONTROL, "private, max-age=31536000, immutable")
                .body(bytes),
            None => Response::builder().status(StatusCode::NOT_FOUND).body(Vec::new()),
        };
        responder.respond(response.unwrap_or_else(|_| Response::new(Vec::new())));
    });
}
