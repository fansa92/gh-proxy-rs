use worker::*;

/// 首页 HTML
///
/// HTML 实际文件位于：
///
///     static/index.html
///
/// 使用 include_str! 在编译 WASM 时直接嵌入 Worker。
const INDEX_HTML: &str =
    include_str!("../static/index.html");

/// 返回首页
pub fn index() -> Result<Response> {
    let headers = Headers::new();

    headers.set(
        "content-type",
        "text/html; charset=UTF-8",
    )?;

    headers.set(
        "cache-control",
        "public, max-age=3600",
    )?;

    headers.set(
        "x-content-type-options",
        "nosniff",
    )?;

    Ok(
        Response::ok(INDEX_HTML)?
            .with_headers(headers)
    )
}