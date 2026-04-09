### hi yes this is an mcp server that runs in rust and provides tools to claude code
### setting up something like this allows you to give claude direct access to tooling on your system
### there is an included city bus routing tool as a test for anyone interested in playing with that

# mcp-tool-server

MCP stdio server that exposes configurable tools to Claude Code.

## Build & run

```sh
cargo build --release
./target/release/mcp-tool-server config.toml
```

## Wire into Claude Code

Add to `.mcp.json` in your project or `~/.claude/mcp.json` globally:

```json
{
  "mcpServers": {
    "local-tools": {
      "command": "/home/wood/src/mcp-tool-server/target/release/mcp-tool-server",
      "args": ["/home/wood/src/mcp-tool-server/config.toml"]
    }
  }
}
```

## config.toml

### Exec tool — runs a subprocess

```toml
[tools.my_tool]
kind         = "exec"
description  = "Shown to Claude so it knows when to use this tool"
executable   = "/usr/bin/my-program"
base_args    = ["--flag", "{param_name}"]   # {placeholders} filled by Claude
working_dir  = "/some/dir"                  # optional
timeout_secs = 30                           # optional, default 30

[tools.my_tool.env]                         # optional
MY_VAR = "value"

[[tools.my_tool.params]]
name        = "param_name"
description = "What this param does"
required    = true

[[tools.my_tool.params]]
name        = "optional_param"
description = "Has a fallback"
required    = false
default     = "fallback"
```

### Value tool — returns a static string

```toml
[tools.my_value]
kind        = "value"
description = "Returns the project root"
value       = "/home/wood/src"
```

## Notes

- Subprocess stdout + stderr are both captured and returned to Claude.
- Non-zero exit codes are surfaced as `[exit N]` in the output, not as errors, so Claude can reason about failures.
- Reload by restarting the server process (or reconnecting in Claude Code with `/mcp`).
