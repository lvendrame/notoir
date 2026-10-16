//! `notoir mcp-bridge --token <token> [--port <p>]` (docs/specs/29
//! §Transport): speaks MCP over stdio (one JSON-RPC message per line) for
//! stdio-only clients and forwards each message to the running app's HTTP
//! endpoint. It never starts Notoir: with nothing listening, requests get the
//! error "Notoir is not running".

use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

use serde_json::{Value, json};

pub const NOT_RUNNING: &str = "Notoir is not running";

pub struct Bridge {
    url: String,
    token: String,
    agent: ureq::Agent,
    session: Option<String>,
    protocol: Option<String>,
}

impl Bridge {
    pub fn new(token: &str, port: u16) -> Bridge {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(3)))
            .timeout_global(None)
            .build()
            .into();
        Bridge {
            url: format!("http://127.0.0.1:{port}/mcp"),
            token: token.to_owned(),
            agent,
            session: None,
            protocol: None,
        }
    }

    /// Runs until `input` ends, then closes the session.
    pub fn run(&mut self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            for reply in self.forward(&line) {
                writeln!(output, "{reply}")?;
                output.flush()?;
            }
        }
        if let Some(session) = &self.session {
            let _ = self
                .agent
                .delete(&self.url)
                .header("Authorization", &format!("Bearer {}", self.token))
                .header("Mcp-Session-Id", session)
                .call();
        }
        Ok(())
    }

    /// Forwards one message; returns the messages to write back.
    pub fn forward(&mut self, line: &str) -> Vec<String> {
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => return vec![error(&Value::Null, -32700, &format!("Parse error: {e}"))],
        };
        let id = msg.get("id").cloned();
        let fail = |code: i64, text: &str| match &id {
            Some(id) => vec![error(id, code, text)],
            None => Vec::new(),
        };
        let mut req = self
            .agent
            .post(&self.url)
            .header("Authorization", &format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");
        if let Some(s) = &self.session {
            req = req.header("Mcp-Session-Id", s);
        }
        if let Some(p) = &self.protocol {
            req = req.header("MCP-Protocol-Version", p);
        }
        let mut resp = match req.send(line) {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(error = %e, "bridge: request failed");
                return fail(-32000, NOT_RUNNING);
            }
        };
        let status = resp.status().as_u16();
        if let Some(s) = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            self.session = Some(s.to_owned());
        }
        match status {
            200..=299 => {}
            401 => {
                return fail(
                    -32001,
                    "Notoir rejected the token (missing, invalid or revoked)",
                );
            }
            403 => return fail(-32001, "Notoir refused the request"),
            404 if self.session.is_some() => {
                // The session expired (server restarted): start over.
                self.session = None;
                return fail(-32000, "The Notoir MCP session ended; reconnect");
            }
            s => return fail(-32000, &format!("Notoir answered HTTP {s}")),
        }
        let sse = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|c| c.starts_with("text/event-stream"));
        let mut out = Vec::new();
        if sse {
            let reader = BufReader::new(resp.body_mut().as_reader());
            let mut data = String::new();
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if let Some(d) = line.strip_prefix("data:") {
                    data.push_str(d.trim_start());
                } else if line.is_empty() && !data.is_empty() {
                    out.extend(self.message(&std::mem::take(&mut data)));
                }
            }
            if !data.is_empty() {
                out.extend(self.message(&data));
            }
        } else if let Ok(body) = resp.body_mut().read_to_string()
            && !body.trim().is_empty()
        {
            out.extend(self.message(&body));
        }
        out
    }

    /// One JSON message from the server, compacted to a single line.
    fn message(&mut self, data: &str) -> Option<String> {
        let v: Value = serde_json::from_str(data).ok()?;
        if let Some(p) = v.pointer("/result/protocolVersion").and_then(Value::as_str) {
            self.protocol = Some(p.to_owned());
        }
        Some(v.to_string())
    }
}

fn error(id: &Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

/// Parses `--token <t> [--port <p>]`.
pub fn parse_args(args: &[String]) -> Result<(String, u16), String> {
    let mut token = None;
    let mut port = crate::DEFAULT_PORT;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--token" => token = it.next().cloned(),
            "--port" => {
                port = it
                    .next()
                    .and_then(|p| p.parse().ok())
                    .ok_or("--port needs a number")?;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let token = token
        .or_else(|| std::env::var("NOTOIR_MCP_TOKEN").ok())
        .ok_or("--token is required")?;
    Ok((token, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn args() {
        assert_eq!(
            parse_args(&s(&["--token", "t", "--port", "5"])),
            Ok(("t".into(), 5))
        );
        assert_eq!(
            parse_args(&s(&["--token", "t"])),
            Ok(("t".into(), crate::DEFAULT_PORT))
        );
        assert!(parse_args(&s(&["--port", "x", "--token", "t"])).is_err());
    }

    // spec 29: "Bridge without running app"
    #[test]
    fn bridge_without_running_app() {
        // A port nothing listens on.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut bridge = Bridge::new("ntr_x", port);
        let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
        let mut out = Vec::new();
        bridge
            .run(std::io::Cursor::new(format!("{init}\n")), &mut out)
            .unwrap();
        let reply: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["error"]["message"], NOT_RUNNING);
        // Notifications get no answer.
        assert!(
            bridge
                .forward(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .is_empty()
        );
    }
}
