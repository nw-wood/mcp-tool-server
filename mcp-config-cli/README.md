# mcp-config-cli

Command-line tool for managing an `mcp-tool-server` config file.

## Build

```sh
cargo build --release
# binary: ./target/release/mcp-config
```

## Usage

All commands accept `--config <path>` (default: `./config.toml`).

```sh
# List all tools
mcp-config list

# Add an exec tool
mcp-config add-exec greet \
  --description "Say hello" \
  --executable echo \
  --arg "hello {name}"

# Add a parameter to an exec tool
mcp-config add-param greet \
  --name name \
  --description "Name to greet" \
  --required

# Add an exec tool with env vars and working dir
mcp-config add-exec build \
  --description "Build a cargo package" \
  --executable cargo \
  --arg build --arg --package --arg "{package}" \
  --env RUST_LOG=info \
  --working-dir /home/wood/src \
  --timeout-secs 120

# Add a value tool
mcp-config add-value project_root \
  --description "Returns the project root path" \
  --value "/home/wood/src"

# Show full TOML for one tool
mcp-config show greet

# Remove a tool
mcp-config remove greet
```

## Notes

- Uses `toml_edit` internally — edits preserve existing comments and formatting.
- Changes are written immediately; no separate save step.
- The MCP server must be restarted to pick up changes.
