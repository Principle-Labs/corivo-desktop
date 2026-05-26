//! Single-shot loopback HTTP server. Accepts the OAuth callback,
//! parses `code` + `state`, 302-redirects the browser to a branded
//! success / error page, and hands back the parsed values.

use std::collections::HashMap;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

use crate::error::{CorivoError, Result};

/// Wait for one HTTP GET to land on the loopback listener. Returns
/// `(code, state)` on success, or surfaces the upstream error
/// (`?error=access_denied` etc.) as `CorivoError::Internal`.
///
/// Random pokes (favicon, port scanners) get a 404 and we keep waiting.
/// We deliberately don't redirect random GETs to the success page — that
/// would turn the loopback into a tiny open redirector.
pub(super) async fn wait_for_callback(
    listener: &TcpListener,
    callback_path: &str,
    success_url: &str,
    error_url_prefix: &str,
) -> Result<(String, String)> {
    loop {
        let (mut socket, _) = listener
            .accept()
            .await
            .map_err(|e| CorivoError::Internal(format!("loopback accept: {e}")))?;

        // 4 KiB is plenty — the wire payload is just
        // `GET /callback?code=…&state=… HTTP/1.1\r\n…`.
        let mut buf = [0u8; 4096];
        let n = socket
            .read(&mut buf)
            .await
            .map_err(|e| CorivoError::Internal(format!("loopback read: {e}")))?;
        let request = String::from_utf8_lossy(&buf[..n]);

        let request_line = request.lines().next().unwrap_or("");
        let path = match request_line.split_whitespace().nth(1) {
            Some(p) => p,
            None => {
                let _ = write_status(&mut socket, "404 Not Found").await;
                continue;
            }
        };
        if !path.starts_with(callback_path) {
            let _ = write_status(&mut socket, "404 Not Found").await;
            continue;
        }

        // Parse the query as a full URL; host is irrelevant, we just
        // want url crate's percent-decoding.
        let parsed = match Url::parse(&format!("http://127.0.0.1{path}")) {
            Ok(u) => u,
            Err(_) => {
                let _ = write_redirect(
                    &mut socket,
                    &error_url(error_url_prefix, "invalid_callback"),
                )
                .await;
                continue;
            }
        };
        let params: HashMap<String, String> = parsed
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();

        if let Some(err) = params.get("error") {
            let detail = params
                .get("error_description")
                .cloned()
                .unwrap_or_else(|| err.clone());
            let _ = write_redirect(&mut socket, &error_url(error_url_prefix, &detail)).await;
            return Err(CorivoError::Internal(format!(
                "OAuth provider denied authorization: {detail}"
            )));
        }

        let code = match params.get("code") {
            Some(c) => c.clone(),
            None => {
                let _ =
                    write_redirect(&mut socket, &error_url(error_url_prefix, "missing_code")).await;
                continue;
            }
        };
        let state = match params.get("state") {
            Some(s) => s.clone(),
            None => {
                let _ = write_redirect(&mut socket, &error_url(error_url_prefix, "missing_state"))
                    .await;
                continue;
            }
        };

        let _ = write_redirect(&mut socket, success_url).await;
        return Ok((code, state));
    }
}

fn error_url(prefix: &str, reason: &str) -> String {
    let encoded: String = url::form_urlencoded::byte_serialize(reason.as_bytes()).collect();
    format!("{prefix}?reason={encoded}")
}

/// 302 Found with empty body. 302 (not 303) is what every OAuth
/// implementation in the wild uses for this exact purpose.
async fn write_redirect(socket: &mut tokio::net::TcpStream, location: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    socket.write_all(response.as_bytes()).await?;
    socket.shutdown().await?;
    Ok(())
}

async fn write_status(socket: &mut tokio::net::TcpStream, status: &str) -> std::io::Result<()> {
    let response = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    socket.write_all(response.as_bytes()).await?;
    socket.shutdown().await?;
    Ok(())
}
