use worker::*;
mod web;

const PREFIX: &str = "/";

// 是否启用 jsDelivr
const JSDELIVR_ENABLED: bool = false;

// 白名单
const WHITE_LIST: &[&str] = &[];

// 文件大小限制，0 = 不限制
const SIZE_LIMIT: u64 = 0;

// 禁止访问
const BANNED_LIST: &[&str] = &[];

// 允许访问
const ALLOWED_LIST: &[&str] = &[];

// 直接放行
const PASSBY_LIST: &[&str] = &[];


// ============================================================
// CORS
// ============================================================

fn cors_headers() -> Result<Headers> {
    let headers = Headers::new();

    headers.set("access-control-allow-origin", "*")?;

    headers.set(
        "access-control-allow-methods",
        "GET,HEAD,POST,PUT,PATCH,DELETE,OPTIONS",
    )?;

    headers.set(
        "access-control-allow-headers",
        "*",
    )?;

    headers.set(
        "access-control-expose-headers",
        "*",
    )?;

    Ok(headers)
}


// ============================================================
// 简单文本响应
// ============================================================

fn text_response(
    text: &str,
    status: u16,
) -> Result<Response> {
    let headers = cors_headers()?;

    Ok(Response::ok(text)?
        .with_headers(headers)
        .with_status(status))
}


// ============================================================
// URL 中提取 GitHub 用户/仓库
// ============================================================

fn extract_user_repo(url: &str) -> Option<String> {
    let url = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");

    let path = if let Some(path) =
        url.strip_prefix("github.com/")
    {
        path
    } else if let Some(path) =
        url.strip_prefix("raw.githubusercontent.com/")
    {
        path
    } else {
        return None;
    };

    let parts: Vec<&str> =
        path.split('/').collect();

    if parts.len() >= 2 {
        Some(format!(
            "github.com/{}/{}",
            parts[0],
            parts[1]
        ))
    } else {
        None
    }
}


// ============================================================
// 用户 / 仓库访问控制
// ============================================================

fn check_access(
    url: &str,
) -> (bool, bool) {
    let user_repo =
        match extract_user_repo(url) {
            Some(value) => value,
            None => return (true, false),
        };

    // PASSBY
    for item in PASSBY_LIST {
        if user_repo.starts_with(item) {
            return (true, true);
        }
    }

    // BANNED
    for item in BANNED_LIST {
        if user_repo.starts_with(item) {
            return (false, false);
        }
    }

    // ALLOWED
    if !ALLOWED_LIST.is_empty() {
        for item in ALLOWED_LIST {
            if user_repo.starts_with(item) {
                return (true, false);
            }
        }

        return (false, false);
    }

    (true, false)
}


// ============================================================
// WhiteList
// ============================================================

fn check_whitelist(
    url: &str,
) -> bool {
    if WHITE_LIST.is_empty() {
        return true;
    }

    WHITE_LIST
        .iter()
        .any(|item| url.contains(item))
}


// ============================================================
// 判断是否为 GitHub URL
// ============================================================

fn is_github_url(
    url: &str,
) -> bool {
    url.starts_with(
        "https://github.com/",
    )
    || url.starts_with(
        "http://github.com/",
    )
    || url.starts_with(
        "https://raw.githubusercontent.com/",
    )
    || url.starts_with(
        "http://raw.githubusercontent.com/",
    )
    || url.starts_with(
        "https://gist.githubusercontent.com/",
    )
    || url.starts_with(
        "http://gist.githubusercontent.com/",
    )
}


// ============================================================
// HEAD 检查文件大小
// ============================================================

async fn check_size(
    url: &str,
) -> Result<Option<Response>> {
    if SIZE_LIMIT == 0 {
        return Ok(None);
    }

    let target =
        Url::parse(url)?;

    let mut init =
        RequestInit::new();

    init.with_method(Method::Head);

    let request =
        Request::new_with_init(
            target.as_str(),
            &init,
        )?;

    let mut response =
        match Fetch::Request(request)
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => {
                return Err(err);
            }
        };
    if let Some(length) =
        response
            .headers()
            .get("content-length")?
    {
        if let Ok(length) =
            length.parse::<u64>()
        {
            if length > SIZE_LIMIT {
                return Ok(Some(
                    Response::redirect(
                        target,
                    )?,
                ));
            }
        }
    }

    Ok(None)
}


// ============================================================
// 核心代理
// ============================================================
//
// ★ 最重要的修改：
//
// 原来的代码:
//
// let body = response.bytes().await?;
// Response::from_bytes(body)
//
// 会把整个 ZIP / EXE / TAR 等文件一次性读取到内存。
//
// 现在:
//
// let body = response.stream()?;
// Response::from_stream(body)
//
// 使用 Worker ReadableStream 流式传输。
// ============================================================

async fn proxy(
    url: Url,
    req: Request,
) -> Result<Response> {

    let method =
        req.method();

    let mut init =
        RequestInit::new();

    init.with_method(method);

    // =========================
    // 上游请求 Header 白名单
    // =========================

    let mut upstream_headers =
        Headers::new();

    // Range：大文件/断点续传必须保留
    if let Some(range) =
        req.headers().get("Range")?
    {
        upstream_headers.set(
            "Range",
            &range,
        )?;
    }

    // If-Range
    if let Some(if_range) =
        req.headers().get("If-Range")?
    {
        upstream_headers.set(
            "If-Range",
            &if_range,
        )?;
    }

    // ETag
    if let Some(if_none_match) =
        req.headers().get("If-None-Match")?
    {
        upstream_headers.set(
            "If-None-Match",
            &if_none_match,
        )?;
    }

    // Last-Modified
    if let Some(if_modified_since) =
        req.headers().get("If-Modified-Since")?
    {
        upstream_headers.set(
            "If-Modified-Since",
            &if_modified_since,
        )?;
    }

    // User-Agent
    if let Some(user_agent) =
        req.headers().get("User-Agent")?
    {
        upstream_headers.set(
            "User-Agent",
            &user_agent,
        )?;
    } else {
        upstream_headers.set(
            "User-Agent",
            "gh-proxy-rs",
        )?;
    }

    // Accept
    if let Some(accept) =
        req.headers().get("Accept")?
    {
        upstream_headers.set(
            "Accept",
            &accept,
        )?;
    } else {
        upstream_headers.set(
            "Accept",
            "*/*",
        )?;
    }

    init.with_headers(
        upstream_headers,
    );

    // 自动跟随 GitHub Release 302
    init.with_redirect(
        RequestRedirect::Follow,
    );

    let upstream =
        Request::new_with_init(
            url.as_str(),
            &init,
        )?;

    let mut response =
        match Fetch::Request(upstream)
            .send()
            .await
        {
            Ok(response) => response,

            Err(err) => {
                return Response::error(
                    format!(
                        "upstream fetch error: {}",
                        err
                    ),
                    500,
                );
            }
        };

    let response_headers =
        response.headers().clone();

    response_headers.set(
        "access-control-allow-origin",
        "*",
    )?;

    response_headers.set(
        "cache-control",
        "public, max-age=31536000, immutable",
    )?;

    response_headers.set(
        "access-control-expose-headers",
        "*",
    )?;

    response_headers.delete(
        "content-security-policy",
    )?;

    response_headers.delete(
        "content-security-policy-report-only",
    )?;

    response_headers.delete(
        "clear-site-data",
    )?;

    let status =
        response.status_code();

    let body = response.bytes().await?;

    Ok(
        Response::from_bytes(body)?
            .with_headers(response_headers)
            .with_status(status)
    )
}


// ============================================================
// OPTIONS
// ============================================================

async fn options_response()
    -> Result<Response>
{
    let headers =
        cors_headers()?;

    Ok(
        Response::empty()?
            .with_headers(headers)
            .with_status(204),
    )
}


// ============================================================
// Worker
// ============================================================

#[event(fetch)]
async fn main(
    req: Request,
    _env: Env,
    _ctx: Context,
) -> Result<Response> {

    // --------------------------------------------------------
    // OPTIONS
    // --------------------------------------------------------

    if req.method()
        == Method::Options
    {
        return options_response()
            .await;
    }

    // --------------------------------------------------------
    // Worker URL
    // --------------------------------------------------------

    let worker_url =
        req.url()?;

    let pathname =
        worker_url.path();

    // --------------------------------------------------------
    // 根目录
    // --------------------------------------------------------

    if pathname == "/"
        || pathname.is_empty()
    {
        return web::index();
    }

    // --------------------------------------------------------
    // PREFIX
    // --------------------------------------------------------

    let target =
        pathname
            .strip_prefix(PREFIX)
            .unwrap_or(pathname);

    // --------------------------------------------------------
    // 去掉开头 /
    // --------------------------------------------------------

    let mut target =
        target
            .trim_start_matches('/')
            .to_string();

    // --------------------------------------------------------
    // URL 解码
    // --------------------------------------------------------

    if target.starts_with(
        "https:/",
    )
    && !target.starts_with(
        "https://",
    ) {
        target =
            target.replacen(
                "https:/",
                "https://",
                1,
            );
    }

    if target.starts_with(
        "http:/",
    )
    && !target.starts_with(
        "http://",
    ) {
        target =
            target.replacen(
                "http:/",
                "http://",
                1,
            );
    }

    // --------------------------------------------------------
    // 没有协议时默认 HTTPS
    // --------------------------------------------------------

    if !target.starts_with(
        "http://",
    )
    && !target.starts_with(
        "https://",
    ) {
        target =
            format!(
                "https://{}",
                target,
            );
    }

    // --------------------------------------------------------
    // URL 检查
    // --------------------------------------------------------

    if !is_github_url(
        &target,
    ) {
        return text_response(
            "Invalid GitHub URL",
            400,
        );
    }

    // --------------------------------------------------------
    // 白名单
    // --------------------------------------------------------

    if !check_whitelist(
        &target,
    ) {
        return text_response(
            "Blocked by whitelist",
            403,
        );
    }

    // --------------------------------------------------------
    // 用户 / Repo 权限
    // --------------------------------------------------------

    let (
        allowed,
        passby,
    ) =
        check_access(
            &target,
        );

    if !allowed {
        return text_response(
            "User/repository access denied",
            403,
        );
    }

    // --------------------------------------------------------
    // PASSBY
    // --------------------------------------------------------

    if passby {
        let target_url =
            Url::parse(&target)?;

        return Ok(
            Response::redirect(
                target_url,
            )?,
        );
    }

    // --------------------------------------------------------
    // 文件大小检查
    // --------------------------------------------------------

    if let Some(response) =
        check_size(&target)
            .await?
    {
        return Ok(response);
    }

    // --------------------------------------------------------
    // 目标 URL
    // --------------------------------------------------------

    let target_url =
        Url::parse(&target)?;

    // --------------------------------------------------------
    // 代理
    // --------------------------------------------------------

    proxy(
        target_url,
        req,
    )
    .await
}