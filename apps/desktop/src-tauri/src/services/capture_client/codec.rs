//! NDJSON line codec for the capture-helper protocol.
//!
//! The wire format is one JSON object per line, terminated by `\n`. Each
//! line is at most [`MAX_LINE_BYTES`] — we cap the read so a misbehaving
//! helper that emits an unbounded line can't OOM us.

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

use super::protocol::Message;

/// Hard cap on a single NDJSON line (1 MiB). Chosen large enough to fit any
/// realistic AX walk output (`max_chars: 64000` in spec) plus envelope
/// overhead, with margin.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

#[derive(thiserror::Error, Debug)]
pub enum CodecError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// Helper sent a line that exceeded [`MAX_LINE_BYTES`] before its
    /// terminating newline. Connection should be considered poisoned —
    /// caller drops the helper.
    #[error("line exceeded {} bytes without newline", MAX_LINE_BYTES)]
    LineTooLong,

    /// Helper-emitted line was not parseable as JSON / didn't match
    /// the [`Message`] envelope.
    #[error("invalid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),

    /// stdout closed cleanly (helper exited).
    #[error("eof")]
    Eof,
}

/// Read one NDJSON message from `reader`. Reuses `line_buf` across calls so
/// no allocation churn in the hot path.
pub async fn read_message<R>(reader: &mut R, line_buf: &mut Vec<u8>) -> Result<Message, CodecError>
where
    R: AsyncBufRead + Unpin,
{
    line_buf.clear();
    read_line_capped(reader, line_buf, MAX_LINE_BYTES).await?;
    // Strip the trailing '\n' (and an optional '\r' on Windows pipes).
    let mut len = line_buf.len();
    if len > 0 && line_buf[len - 1] == b'\n' {
        len -= 1;
    }
    if len > 0 && line_buf[len - 1] == b'\r' {
        len -= 1;
    }
    let msg: Message = serde_json::from_slice(&line_buf[..len])?;
    Ok(msg)
}

/// Read bytes from `reader` until either a `\n` is consumed or `limit`
/// bytes have been buffered without one (which is an error).
async fn read_line_capped<R>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    limit: usize,
) -> Result<(), CodecError>
where
    R: AsyncBufRead + Unpin,
{
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            if buf.is_empty() {
                return Err(CodecError::Eof);
            }
            // Treat trailing partial line (no newline) as EOF — helper
            // shouldn't end this way, but if it does the unterminated
            // line is unusable.
            return Err(CodecError::Eof);
        }
        if let Some(pos) = chunk.iter().position(|&b| b == b'\n') {
            let take = pos + 1; // include the newline
            if buf.len() + take > limit {
                return Err(CodecError::LineTooLong);
            }
            buf.extend_from_slice(&chunk[..take]);
            reader.consume(take);
            return Ok(());
        } else {
            if buf.len() + chunk.len() > limit {
                return Err(CodecError::LineTooLong);
            }
            buf.extend_from_slice(chunk);
            let consumed = chunk.len();
            reader.consume(consumed);
        }
    }
}

/// Serialize and write one NDJSON message. Writes a single `\n`-terminated
/// line and flushes — the writer must be unbuffered on the helper side or
/// it'll deadlock on response.
pub async fn write_message<W>(writer: &mut W, message: &Message) -> Result<(), CodecError>
where
    W: AsyncWrite + Unpin,
{
    let mut bytes = serde_json::to_vec(message)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

/// Convenience: helper-side / mock-side `read_message_from_stdin` that
/// applies the same line cap. Used by the mock helper bin to keep its
/// IPC core symmetric with the client.
pub async fn read_message_from<R>(reader: R) -> Result<(Message, R, Vec<u8>), CodecError>
where
    R: AsyncBufRead + Unpin,
{
    let mut buf = Vec::with_capacity(4096);
    let mut reader = reader;
    let msg = read_message(&mut reader, &mut buf).await?;
    Ok((msg, reader, buf))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn round_trip_through_pipe() {
        let original = Message::HelloAck(super::super::protocol::HelloAck {
            selected_protocol: super::super::protocol::ProtocolVersion::V1,
            client_version: "0.1.0".into(),
        });

        let mut wire = Vec::new();
        write_message(&mut wire, &original).await.unwrap();
        assert!(wire.last() == Some(&b'\n'));

        let mut reader = BufReader::new(&wire[..]);
        let mut buf = Vec::new();
        let parsed = read_message(&mut reader, &mut buf).await.unwrap();
        match parsed {
            Message::HelloAck(_) => {}
            other => panic!("expected HelloAck, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reads_multiple_messages_in_sequence() {
        let mut wire = Vec::new();
        for v in ["a", "b", "c"] {
            let msg = Message::Log(super::super::protocol::LogMessage {
                level: super::super::protocol::LogLevel::Info,
                message: v.into(),
                target: None,
                fields: None,
            });
            write_message(&mut wire, &msg).await.unwrap();
        }
        let mut reader = BufReader::new(&wire[..]);
        let mut buf = Vec::new();
        for expected in ["a", "b", "c"] {
            match read_message(&mut reader, &mut buf).await.unwrap() {
                Message::Log(l) => assert_eq!(l.message, expected),
                other => panic!("expected Log, got {other:?}"),
            }
        }
        // EOF after all consumed
        match read_message(&mut reader, &mut buf).await {
            Err(CodecError::Eof) => {}
            other => panic!("expected EOF, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_oversized_line() {
        let mut huge = Vec::with_capacity(MAX_LINE_BYTES + 16);
        huge.extend(std::iter::repeat(b'x').take(MAX_LINE_BYTES + 1));
        huge.push(b'\n');
        let mut reader = BufReader::new(&huge[..]);
        let mut buf = Vec::new();
        match read_message(&mut reader, &mut buf).await {
            Err(CodecError::LineTooLong) => {}
            other => panic!("expected LineTooLong, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn handles_crlf_line_ending() {
        let mut wire = Vec::new();
        let msg_json = json!({
            "type": "heartbeat",
            "ts": "2026-05-05T12:00:00Z"
        })
        .to_string();
        wire.extend_from_slice(msg_json.as_bytes());
        wire.extend_from_slice(b"\r\n");

        let mut reader = BufReader::new(&wire[..]);
        let mut buf = Vec::new();
        match read_message(&mut reader, &mut buf).await.unwrap() {
            Message::Heartbeat(_) => {}
            other => panic!("expected Heartbeat, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn invalid_json_returns_invalid_json_error() {
        let wire = b"not even json\n".to_vec();
        let mut reader = BufReader::new(&wire[..]);
        let mut buf = Vec::new();
        match read_message(&mut reader, &mut buf).await {
            Err(CodecError::InvalidJson(_)) => {}
            other => panic!("expected InvalidJson, got {other:?}"),
        }
    }
}
