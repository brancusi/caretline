# caretline-mcp

An [MCP](https://modelcontextprotocol.io) server that lets an agent read and edit a
[caretline](../../docs/README.md) editor: a live one a person is typing in
(`caretline FILE --listen`), or a headless engine on a file. Writes are guarded by the
document's revision, the agent gets its own caret, edits are attributed, and the session's
trace replays exactly.

```sh
cargo install --locked --path crates/caretline-mcp
claude mcp add caretline -- caretline-mcp
caretline notes.md --listen        # then ask the agent to edit the file you have open
```

Tools: `list_editors`, `open`, `read`, `edit`, `view_open`, `view_close`, `watch`, `trace`,
`save`, `close`. See [docs/mcp.md](../../docs/mcp.md) for setup in Claude
Code, Claude Desktop and Cursor, the tool reference, a worked example and the threat model.

`cargo run -p caretline-mcp --example agent_session` runs a scripted agent against the newest
live editor.

MIT.
