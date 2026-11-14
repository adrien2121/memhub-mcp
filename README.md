# memhub-mcp

Persistent note memory for AI agents over MCP: write, read, edit, delete, list and
search notes. Search is hybrid: keyword plus semantic, with a local embedding model.

## Install

```sh
cargo build --release
cp target/release/memhub-mcp ~/.local/bin/   # or anywhere on PATH
memhub-mcp setup
```

`setup` downloads the bge-small-en-v1.5 int8 model, its tokenizer and the onnxruntime
1.31.0 library into the data dir. Every file is pinned by SHA-256; re-running is safe.

## Connect it to your AI tool

`memhub-mcp setup` connects memhub-mcp to your AI tools for you. It looks for Claude Code,
Codex, Gemini CLI, Cursor, omp, Claude Desktop and VS Code.

When both input and output are terminals, `setup` shows a list of these tools. The tools it found are checked. Press
Enter to accept, or type a tool's name to check or uncheck it. If you check a tool that was
not found, `setup` asks where it is. Then `setup` asks:
"Also tell your AI tools to use memhub-mcp? [Y/n]". Yes adds a short rule to each tool's
global instruction file, so the AI searches its notes before a task and saves new ones:

| Tool | Instruction file |
|---|---|
| Claude Code | `~/.claude/CLAUDE.md` |
| Codex | `~/.codex/AGENTS.md` (or `$CODEX_HOME/AGENTS.md`) |
| Gemini CLI | `~/.gemini/GEMINI.md` |
| omp | `~/.omp/agent/AGENTS.md` |

The rule sits between `<!-- memhub-mcp:start -->` and `<!-- memhub-mcp:end -->`. Running
`setup` again updates it and never adds a second copy. If the file already talks about
memhub-mcp, `setup` leaves it alone. Cursor, Claude Desktop and VS Code have no such file:
`setup` prints the rule for you to paste into their settings.

In a script (no terminal), `setup` connects every tool it finds and asks nothing. Add
`--rules` to also add the rule. Use `--yes` to skip the questions in a terminal too (this
connects every tool found and adds the rule).

`setup` prints one line per tool; then restart your AI tools. It never changes a
memhub-mcp entry you already have, and it saves a copy of each file it changes as
`<file>.bak`. Installed a new AI tool later? Run `memhub-mcp setup` again.

If `setup` says "not found" for a tool you have, or you use a variant such as VS Code
Insiders, run `setup` again and tell it where the tool is with `--path <name>=<path>`. For
Claude Code and Codex, give the program; for the others, give the settings file. Example
for VS Code Insiders on macOS:

```sh
memhub-mcp setup --path vscode="$HOME/Library/Application Support/Code - Insiders/User/mcp.json"
```

Names: `claude-code`, `codex`, `gemini`, `cursor`, `omp`, `claude-desktop`, `vscode`.

## Manual setup (if automatic setup failed)

If `setup` says "failed" for a tool, find your tool below and do what it says, then
restart the tool.

### Claude Code

Run in a terminal:

```sh
claude mcp add memhub-mcp -- memhub-mcp
```

### Codex

Run in a terminal:

```sh
codex mcp add memhub-mcp -- memhub-mcp
```

### Gemini CLI, Cursor, omp

Open the settings file for your tool in a text editor (create it if it doesn't exist):

- Gemini CLI: `~/.gemini/settings.json`
- Cursor: `~/.cursor/mcp.json`
- omp: `~/.omp/agent/mcp.json`

`~` means your home folder. If the file is new or empty, paste this in:

```json
{
  "mcpServers": {
    "memhub-mcp": { "command": "memhub-mcp" }
  }
}
```

If the file already lists other tools under `"mcpServers"`, add memhub-mcp as the
last entry. Put a comma after the entry above it, and no comma after yours:

```json
{
  "mcpServers": {
    "some-other-tool": { "command": "some-other-tool" },
    "memhub-mcp": { "command": "memhub-mcp" }
  }
}
```

### Claude Desktop

In Claude Desktop, open Settings → Developer → Edit Config. Paste the same block
as above, but replace the second `memhub-mcp` with the full path to the program,
because the app can't find it otherwise:

```json
{
  "mcpServers": {
    "memhub-mcp": { "command": "/Users/YOUR_NAME/.local/bin/memhub-mcp" }
  }
}
```

To get your exact path, run `which memhub-mcp` in a terminal (macOS, Linux) or
`where memhub-mcp` in Command Prompt (Windows). On Windows, write each `\` in the
path as `\\`, e.g. `"C:\\Users\\YOUR_NAME\\bin\\memhub-mcp.exe"`.

### VS Code

Create or open VS Code's settings file `mcp.json`: `~/Library/Application Support/Code/User/mcp.json`
(macOS), `~/.config/Code/User/mcp.json` (Linux) or `%APPDATA%\Code\User\mcp.json`
(Windows). Put this in it:

```json
{
  "servers": {
    "memhub-mcp": { "type": "stdio", "command": "memhub-mcp" }
  }
}
```

Note the key is `"servers"` here, not `"mcpServers"`.

### Other tools

Any tool that can run a local MCP server works: tell it to run the command
`memhub-mcp` with no arguments.

All your AI tools share the same notes: the first one to start memhub-mcp also
starts a small background process that holds the notes, and the others connect to it.
Other commands: `memhub-mcp daemon`, `memhub-mcp reembed`, `memhub-mcp import <dir>`,
`memhub-mcp backup <path>`.

## Config

Optional `config.toml` in the config dir, overridden by `MEMHUB_MCP_*` env vars
(e.g. `MEMHUB_MCP_THREADS=4`). Keys are documented in `src/config.rs` and
`src/config/embedder.rs`.

| | Data dir | Config dir |
|---|---|---|
| macOS, Linux | `~/.local/share/memhub-mcp` | `~/.config/memhub-mcp` |
| Windows | `%APPDATA%\memhub-mcp` | `%APPDATA%\memhub-mcp` |

## Platforms

`setup` supports macOS arm64, Linux x86_64/aarch64 and Windows x86_64. Elsewhere
(e.g. Intel macOS, which has no official onnxruntime 1.31 build), place the model
files and an onnxruntime 1.31 library yourself and point `model_dir`/`ort_dylib` at them.

## Development

```sh
cargo test
cargo test --release --test latency -- --ignored --nocapture   # fails if search p95 > 20 ms
```
