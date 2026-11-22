# Cursor Agent fixtures

- `not_signed_in.stderr.txt` is real: what `cursor-agent` 2026.08.11 writes to
  stderr, with exit code 1 and nothing on stdout, when nobody is signed in.
- `documented_tool_call.jsonl` is **constructed, not recorded**. No signed-in
  Cursor account was available, so the lines follow Cursor's published
  stream-json reference and the field names the installed CLI uses for MCP
  tool calls, thinking and token usage. Replace it with a real recording
  (account identifiers removed) once someone can run the CLI signed in.
