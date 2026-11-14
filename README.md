# memhub-mcp

Persistent memory for AI agents over MCP. Hybrid keyword + semantic search with a local
embedding model.

## Install

```sh
curl -LsSf https://github.com/adrien2121/memhub-mcp/releases/latest/download/memhub-mcp-installer.sh | sh
memhub-mcp setup
```

Windows (PowerShell):

```powershell
irm https://github.com/adrien2121/memhub-mcp/releases/latest/download/memhub-mcp-installer.ps1 | iex
memhub-mcp setup
```

From source: `cargo install --git https://github.com/adrien2121/memhub-mcp`.

`setup` downloads the model and onnxruntime (SHA-256 pinned), connects memhub-mcp to
Claude Code, Codex, Gemini CLI, Cursor, omp, Claude Desktop and VS Code, and can add a
"use your notes" rule to their instruction files. Safe to re-run; changed files are backed
up as `.bak`. Flags: `memhub-mcp setup --help`.

## Manual setup

Any MCP client works: run `memhub-mcp` (stdio, no arguments).

```json
{ "mcpServers": { "memhub-mcp": { "command": "memhub-mcp" } } }
```

VS Code uses `"servers"` instead of `"mcpServers"`. Claude Desktop needs the full path
(`which memhub-mcp`; Windows: `where.exe memhub-mcp`).

## Commands

`setup` · `import <dir>` · `backup <path>` · `reembed` · `daemon`

All clients share one background daemon and one database.

## Config

Optional `config.toml` in the config dir, overridden by `MEMHUB_MCP_*` env vars.

| | Data | Config |
|---|---|---|
| macOS, Linux | `~/.local/share/memhub-mcp` | `~/.config/memhub-mcp` |
| Windows | `%APPDATA%\memhub-mcp` | `%APPDATA%\memhub-mcp` |

`setup` supports macOS arm64, Linux x86_64/aarch64 and Windows x86_64. Elsewhere, provide
the model and onnxruntime 1.31 yourself and set `model_dir` / `ort_dylib`.
