mod config;
mod protocol;
mod tool;

use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};

use config::Config;
use protocol::{Request, Response};
use tool::ToolRegistry;

// Global log file, written from the sync macro below.
static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

macro_rules! log {
    ($($arg:tt)*) => {{
        let msg = format!($($arg)*);
        // Always write to stderr (captured by Claude Code's debug log).
        eprintln!("{}", msg);
        // Also append to the log file if one is open.
        if let Ok(mut guard) = LOG.lock() {
            if let Some(ref mut f) = *guard {
                use std::io::Write;
                let _ = writeln!(f, "{}", msg);
            }
        }
    }};
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config_path = std::env::args().nth(1).unwrap_or_else(|| "config.toml".to_string());

    // Derive log path: replace .toml extension with .log, or append .log.
    let log_path = PathBuf::from(&config_path).with_extension("log");
    match std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
        Ok(f) => {
            *LOG.lock().unwrap() = Some(f);
            eprintln!("[mcp-tool-server] Logging to {}", log_path.display());
        }
        Err(e) => {
            eprintln!("[mcp-tool-server] Could not open log file {}: {}", log_path.display(), e);
        }
    }

    let config = Config::load(&config_path)?;
    let registry = ToolRegistry::from_config(config);

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut out = BufWriter::new(tokio::io::stdout());

    log!("[mcp-tool-server] Ready — config: {}", config_path);

    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                log!("[mcp-tool-server] Parse error: {}", e);
                continue;
            }
        };

        if req.method.starts_with("notifications/") {
            log!("[mcp-tool-server] << notification: {}", req.method);
            continue;
        }

        log!("[mcp-tool-server] << {} (id={})", req.method, req.id);

        let response = handle(&registry, req).await;
        let mut json_str = serde_json::to_string(&response)?;
        log!("[mcp-tool-server] >> {} bytes", json_str.len());
        json_str.push('\n');
        out.write_all(json_str.as_bytes()).await?;
        out.flush().await?;
    }

    log!("[mcp-tool-server] stdin closed — exiting");
    Ok(())
}

async fn handle(registry: &ToolRegistry, req: Request) -> Response {
    match req.method.as_str() {
        "initialize" => {
            let client_version = req.params
                .get("protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("2025-03-26");
            log!("[mcp-tool-server] client protocolVersion: {}", client_version);
            Response::ok(
                req.id,
                json!({
                    "protocolVersion": client_version,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "mcp-tool-server", "version": "0.1.0" },
                }),
            )
        }

        "ping" => Response::ok(req.id, json!({})),

        "tools/list" => Response::ok(req.id, registry.list()),

        "tools/call" => {
            let name = req.params["name"].as_str().unwrap_or("").to_string();
            let args = req.params.get("arguments").cloned().unwrap_or(Value::Object(Default::default()));
            log!("[mcp-tool-server] calling tool: {}", name);

            match registry.call(&name, &args).await {
                Ok(text) => Response::ok(
                    req.id,
                    json!({ "content": [{ "type": "text", "text": text }] }),
                ),
                Err(e) => {
                    log!("[mcp-tool-server] tool error: {}", e);
                    Response::err(req.id, -32603, e.to_string())
                }
            }
        }

        method => {
            log!("[mcp-tool-server] unknown method: {}", method);
            Response::err(req.id, -32601, format!("Unknown method: {}", method))
        }
    }
}
