//! MCP over standard input and output, for `moshionly mcp`.
//!
//! The agent starts the process and writes one JSON-RPC message per line
//! to its input; every response goes out as one line on its output.
//! Nothing else may be written to standard output, so diagnostics go to
//! standard error. The server stops when its input ends.

use std::io::{self, BufRead, Write};

use super::McpServer;

/// Serves `server` on this process's standard input and output until the
/// input ends.
pub fn serve(server: &McpServer) -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    serve_lines(server, stdin.lock(), stdout.lock())
}

/// Serves `server` on any pair of streams: one message per line in, one
/// response per line out. Returns when `input` ends.
pub fn serve_lines(
    server: &McpServer,
    mut input: impl BufRead,
    mut output: impl Write,
) -> io::Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let message = line.trim();
        if message.is_empty() {
            continue;
        }
        if let Some(response) = server.handle_text(message) {
            // Compact JSON has no line breaks, so one response is one line.
            output.write_all(response.as_bytes())?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::tools::testing::demo_tools;

    fn run(lines: &[Value]) -> Vec<Value> {
        let input: String = lines.iter().map(|line| format!("{line}\n")).collect();
        let input = format!("{input}\n   \nnot json\n");
        let mut output = Vec::new();
        serve_lines(&McpServer::new(demo_tools()), input.as_bytes(), &mut output).unwrap();
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn a_legacy_conversation_over_lines() {
        let answers = run(&[
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": { "protocolVersion": "2025-11-25", "capabilities": {},
                                "clientInfo": { "name": "t", "version": "1" } } }),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                    "params": { "name": "query_nodes", "arguments": { "type": "text" } } }),
        ]);
        // No answer to the notification or the blank line; one to the junk.
        assert_eq!(answers.len(), 4);
        assert_eq!(answers[0]["result"]["protocolVersion"], "2025-11-25");
        assert!(
            answers[0]["result"]["instructions"]
                .as_str()
                .unwrap()
                .starts_with("You are working in Moshionly")
        );
        assert_eq!(answers[1]["id"], 2);
        assert!(answers[1]["result"]["tools"].is_array());
        let found: Value =
            serde_json::from_str(answers[2]["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(found["nodes"][0]["id"], "title");
        assert_eq!(answers[3]["error"]["code"], -32700);
    }

    #[test]
    fn a_modern_probe_over_lines() {
        let answers = run(&[json!({
            "jsonrpc": "2.0", "id": "d", "method": "server/discover",
            "params": { "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientCapabilities": {} } }
        })]);
        assert_eq!(answers[0]["result"]["supportedVersions"][0], "2026-07-28");
        assert_eq!(answers[0]["result"]["resultType"], "complete");
    }
}
