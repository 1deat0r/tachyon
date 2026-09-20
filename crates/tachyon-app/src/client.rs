//! Gateway client: connects to the local socket, sends one framed
//! request, reads one framed response.
//!
//! The client owns no agent logic; it renders what the gateway returns.

use std::path::Path;

use anyhow::{Context, Result, bail};
use tachyon_gateway::transport::connect;
use tachyon_protocol::{
    Command, CommandResult, RequestEnvelope, ResponseEnvelope, check_version, decode_frame,
    encode_frame,
};
use tachyon_types::EventId;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

/// Sends `command` to the gateway at `address` (socket path or pipe name)
/// and returns the result.
pub async fn send(address: &Path, command: Command) -> Result<CommandResult> {
    let mut stream = connect(address)
        .await
        .with_context(|| format!("connecting to gateway at {}", address.display()))?;
    let request = RequestEnvelope {
        protocol_version: tachyon_protocol::PROTOCOL_VERSION,
        request_id: EventId::generate(),
        command,
    };
    let bytes = encode_frame(&request).context("encoding request")?;
    stream.write_all(&bytes).await.context("sending request")?;
    let mut prefix = [0_u8; tachyon_protocol::FRAME_PREFIX_LEN];
    stream
        .read_exact(&mut prefix)
        .await
        .context("reading response")?;
    let len = u32::from_le_bytes(prefix) as usize;
    if len > tachyon_protocol::MAX_FRAME_BYTES - tachyon_protocol::FRAME_PREFIX_LEN {
        bail!("gateway response exceeds frame limit");
    }
    let mut payload = vec![0_u8; len];
    stream
        .read_exact(&mut payload)
        .await
        .context("reading response")?;
    let mut framed = prefix.to_vec();
    framed.extend_from_slice(&payload);
    let (response, _): (ResponseEnvelope, usize) =
        decode_frame(&framed).context("decoding response")?;
    check_version(response.protocol_version).context("gateway protocol version")?;
    Ok(response.result)
}

/// Renders a command result as pretty JSON on stdout and reports success.
/// Failures additionally log `code: message` on stderr unless `json` mode
/// keeps stdout as the single machine-readable channel.
pub fn render(result: &CommandResult, json: bool) -> Result<bool> {
    match result {
        CommandResult::Ok { .. } => {
            println!("{}", serde_json::to_string_pretty(result)?);
            Ok(true)
        }
        CommandResult::Err { code, message } => {
            if !json {
                eprintln!("tachyon: {code}: {message}");
            }
            println!("{}", serde_json::to_string_pretty(result)?);
            Ok(false)
        }
    }
}
