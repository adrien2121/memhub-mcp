# memhub-mcp

Persistent memory for AI agents over MCP. Hybrid keyword + semantic search with a local
embedding model.

## Install

macOS / Linux:

```sh
curl -LsSf https://github.com/adrien2121/memhub-mcp/releases/latest/download/memhub-mcp-installer.sh | sh
```

Windows (PowerShell):

```powershell
irm https://github.com/adrien2121/memhub-mcp/releases/latest/download/memhub-mcp-installer.ps1 | iex
```

Then, in a new terminal:

```sh
memhub-mcp setup
```

`setup` downloads the model and onnxruntime (SHA-256 pinned), connects memhub-mcp to
Claude Code, Codex, Gemini CLI, Cursor, omp, Claude Desktop and VS Code, and can add a
"use your notes" rule to their instruction files. Safe to re-run; changed files are backed
up as `.bak`. Flags: `memhub-mcp setup --help`.

## Manual setup

Any MCP client works: run `memhub-mcp` (stdio, no arguments). Add this entry to the
client's MCP config file, merging into `mcpServers` if it already exists:

```json
{ "mcpServers": { "memhub-mcp": { "command": "memhub-mcp" } } }
```

| Client | Config file |
|---|---|
| Cursor | `~/.cursor/mcp.json` |
| Gemini CLI | `~/.gemini/settings.json` |
| omp | `~/.omp/agent/mcp.json` |
| Claude Desktop | `<app config>/Claude/claude_desktop_config.json` |
| VS Code | `<app config>/Code/User/mcp.json`, key `"servers"`, add `"type": "stdio"` |
| Claude Code | no file: `claude mcp add --scope user memhub-mcp -- memhub-mcp` |
| Codex | no file: `codex mcp add memhub-mcp -- memhub-mcp` |

`<app config>` is `~/Library/Application Support` on macOS, `%APPDATA%` on Windows and
`~/.config` on Linux. Claude Desktop needs the full path as `command`
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
