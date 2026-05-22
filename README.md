<div align="center">

# CodeGraph

### Rust-native Semantic Code Intelligence for Claude Code, Cursor, Codex, and OpenCode

**~35% cheaper · ~70% fewer tool calls · 100% local**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/Rust-2021-orange.svg)](https://www.rust-lang.org/)
[![SQLite](https://img.shields.io/badge/SQLite-bundled-blue.svg)](https://sqlite.org/)

[![Windows](https://img.shields.io/badge/Windows-supported-blue.svg)](#)
[![macOS](https://img.shields.io/badge/macOS-supported-blue.svg)](#)
[![Linux](https://img.shields.io/badge/Linux-supported-blue.svg)](#)

[![Claude Code](https://img.shields.io/badge/Claude_Code-supported-blueviolet.svg)](#)
[![Cursor](https://img.shields.io/badge/Cursor-supported-blueviolet.svg)](#)
[![Codex CLI](https://img.shields.io/badge/Codex_CLI-supported-blueviolet.svg)](#)
[![opencode](https://img.shields.io/badge/opencode-supported-blueviolet.svg)](#)

<br />

### Get Started

```bash
cd rust
cargo build --release
mkdir -p ~/.local/bin
cp target/release/codegraph-rs ~/.local/bin/codegraph
```

<sub>Builds the Rust CLI and MCP server, then installs it locally as `codegraph`.</sub>

#### Initialize Projects

```bash
cd your-project
codegraph init -i
```

![1_C_VYnhpys0UHrOuOgpgoyw](https://github.com/user-attachments/assets/f168182f-4d9a-44e0-94d7-08d018cc8a3a)

</div>

---

## Why CodeGraph?

When Claude Code explores a codebase, it spawns **Explore agents** that scan files with grep, glob, and Read — consuming tokens on every tool call.

**CodeGraph gives those agents a pre-indexed knowledge graph** — symbol relationships, call graphs, and code structure. Agents query the graph instantly instead of scanning files.

### Benchmark Results

Tested across **7 real-world open-source codebases** spanning 7 languages, comparing an agent (Claude Code, headless) answering one architecture question **with** and **without** CodeGraph. Each cell is the savings at the **median of 4 runs per arm**.

> **Average: 35% cheaper · 59% fewer tokens · 49% faster · 70% fewer tool calls**

| Codebase | Language | Cost | Tokens | Time | Tool calls |
|----------|----------|------|--------|------|------------|
| **VS Code** | TypeScript · ~10k files | 35% cheaper | 73% fewer | 41% faster | 72% fewer |
| **Excalidraw** | TypeScript · ~600 | 47% cheaper | 73% fewer | 60% faster | 86% fewer |
| **Django** | Python · ~2.7k | 34% cheaper | 64% fewer | 59% faster | 81% fewer |
| **Tokio** | Rust · ~700 | 52% cheaper | 81% fewer | 63% faster | 89% fewer |
| **OkHttp** | Java · ~640 | 17% cheaper | 41% fewer | 36% faster | 64% fewer |
| **Gin** | Go · ~150 | 22% cheaper | 23% fewer | 34% faster | 19% fewer |
| **Alamofire** | Swift · ~100 | 38% cheaper | 59% fewer | 51% faster | 77% fewer |

The gains scale with codebase size: on large repos the agent answers from the index in a handful of calls with **zero file reads**, while the no-CodeGraph agent fans out across grep/find/Read (and the sub-agents it spawns). On a small repo like Gin (~150 files) native search is already cheap, so the margin narrows.

<details>
<summary><strong>Full benchmark details</strong></summary>

**Methodology.** Each arm is `claude -p` (Claude Opus 4.7, Claude Code v2.1.145) run headlessly against the repo with `--strict-mcp-config`: **WITH** = CodeGraph's MCP server enabled, **WITHOUT** = an empty MCP config. Built-in Read/Grep/Bash stay available to both. Same question per repo, **4 runs per arm, median reported**. Cost = the run's `total_cost_usd`; Tokens = total tokens processed (input incl. cached + output); Time = wall-clock; Tool calls = every tool invocation, including those inside any sub-agents the model spawns. Repos cloned at `--depth 1` and indexed by the same CodeGraph build that served them.

**Queries:**
| Codebase | Query |
|----------|-------|
| VS Code | "How does the extension host communicate with the main process?" |
| Excalidraw | "How does Excalidraw render and update canvas elements?" |
| Django | "How does Django's ORM build and execute a query from a QuerySet?" |
| Tokio | "How does tokio schedule and run async tasks on its runtime?" |
| OkHttp | "How does OkHttp process a request through its interceptor chain?" |
| Gin | "How does gin route requests through its middleware chain?" |
| Alamofire | "How does Alamofire build, send, and validate a request?" |

**Raw medians — WITH → WITHOUT:**
| Codebase | Cost | Tokens | Time | Tool calls |
|----------|------|--------|------|------------|
| VS Code | $0.42 → $0.64 | 393k → 1.4M | 1m 0s → 1m 43s | 7 → 23 |
| Excalidraw | $0.54 → $1.02 | 851k → 3.2M | 1m 17s → 3m 14s | 12 → 83 |
| Django | $0.41 → $0.62 | 499k → 1.4M | 1m 0s → 2m 25s | 9 → 48 |
| Tokio | $0.50 → $1.04 | 657k → 3.4M | 1m 5s → 2m 56s | 9 → 75 |
| OkHttp | $0.36 → $0.44 | 352k → 596k | 45s → 1m 11s | 5 → 14 |
| Gin | $0.36 → $0.46 | 431k → 562k | 47s → 1m 11s | 7 → 8 |
| Alamofire | $0.61 → $0.99 | 1.1M → 2.6M | 1m 19s → 2m 41s | 15 → 64 |

**Why CodeGraph wins:** with the index available, the agent answers directly — `codegraph_context` to map the area, then one `codegraph_explore` for the relevant source — and stops, usually with zero file reads. Without it, the agent (and the Explore sub-agents it spawns) spends most of its budget on discovery (find/ls/grep) before reading the right code. CodeGraph only helps when queried *directly*, so its instructions steer agents to answer directly rather than delegate exploration to file-reading sub-agents — otherwise a sub-agent reads files regardless and CodeGraph becomes overhead.

</details>

---

## Key Features

| | |
|---|---|
| **Smart Context Building** | One tool call returns entry points, related symbols, and code snippets — no expensive exploration agents |
| **Full-Text Search** | Find code by name instantly across your entire codebase, powered by FTS5 |
| **Impact Analysis** | Trace callers, callees, and the full impact radius of any symbol before making changes |
| **Always Fresh** | The Rust MCP server can run a polling watcher with debounced auto-sync so the graph stays current as you code |
| **Multi-Language Indexing** | Rust AST extraction plus tree-sitter-backed extraction for TypeScript, JavaScript, Python, Go, Java, C, C++, C#, Ruby, PHP, Swift, Lua/Luau, Dart, Scala, Svelte, and Vue |
| **Framework-aware Routes** | Recognizes common web-framework routing files and links URL patterns to their handlers |
| **100% Local** | No data leaves your machine. No API keys. No external services. SQLite database only |

---

## Framework-aware Routes

CodeGraph detects web-framework routing files and emits `route` nodes linked by `references` edges to their handler classes or functions. Querying callers of a view/controller now surfaces the URL pattern that binds it.

| Framework | Shapes recognized |
|---|---|
| **Django** | `path()`, `re_path()`, `url()`, `include()` in `urls.py` (CBV `.as_view()`, dotted paths) |
| **Flask** | `@app.route('/path', methods=[...])`, blueprint routes |
| **FastAPI** | `@app.get(...)`, `@router.post(...)`, all standard methods |
| **Express / Fastify / Koa / Hapi-style JS routers** | `app.get(...)`, `router.post(...)`, `router.route(...)` |
| **Laravel** | `Route::get()`, `Route::post()`, `Route::any()` |
| **Rails** | `get '/x', to: 'users#index'`, `resources`, `namespace` |
| **Spring** | `@GetMapping`, `@PostMapping`, `@RequestMapping` on methods |
| **Gin / chi / Echo / Fiber / net/http** | `r.GET(...)`, `router.HandleFunc(...)` |
| **Axum** | `.route("/x", get(handler))` |
| **ASP.NET** | `[HttpGet("/x")]` attributes on action methods |

---

## Quick Start

### 1. Build the Rust CLI

```bash
cd rust
cargo build --release
mkdir -p ~/.local/bin
cp target/release/codegraph-rs ~/.local/bin/codegraph
```

From source, the compiled binary is `rust/target/release/codegraph-rs`. Put it on your `PATH` as `codegraph`, since generated MCP configs invoke that command.

### 2. Configure Your Agent

```bash
codegraph install --yes                              # auto-detect agents, install global
codegraph install --target=cursor,claude --yes       # explicit target list
codegraph install --target=auto --location=local     # detected agents, project-local
codegraph install --print-config codex               # print snippet, no file writes
```

The installer supports **Claude Code**, **Cursor**, **Codex CLI**, and **opencode**. It writes each selected agent's MCP config and instruction surface, and can set Claude Code auto-allow permissions.

| Flag | Values | Default |
|---|---|---|
| `--target` | `auto`, `all`, `none`, or csv (`claude,cursor,...`) | prompt |
| `--location` | `global`, `local` | prompt |
| `--yes` | (boolean) | prompt every step |
| `--no-permissions` | (boolean) skip Claude auto-allow list | permissions on |
| `--print-config <id>` | dump snippet for one agent and exit | — |
| `--uninstall` | remove generated agent config entries | `false` |

### 3. Restart Your Agent

Restart your agent (Claude Code / Cursor / Codex CLI / opencode) for the MCP server to load.

### 4. Initialize Projects

```bash
cd your-project
codegraph init -i
```

Builds the per-project knowledge graph index under `.codegraph/`. Re-run `codegraph sync` manually after large changes, or let `codegraph serve --mcp` keep the graph current while your agent is connected.

That's it — your agent will use CodeGraph tools automatically when a `.codegraph/` directory exists.

## Rust Implementation

The active Rust implementation lives in [`rust/`](./rust), with detailed migration notes in [`docs/rust-migration.md`](./docs/rust-migration.md). It includes:

- Project lifecycle commands: `init`, `uninit`, `status`, `scan`, `index`, `sync`, `unlock`
- SQLite graph storage via `rusqlite` with bundled SQLite and FTS-backed symbol search
- Rust AST extraction via `syn`
- Tree-sitter-backed extraction for TypeScript, TSX, JavaScript, JSX, Python, Go, Java, C, C++, C#, Ruby, PHP, Swift, Lua/Luau, Dart, and Scala
- Svelte and Vue component extraction with script-block symbol extraction
- Import resolution, tsconfig path aliases, cross-file call resolution, and framework route extraction
- Graph queries for search, callers, callees, impact, file lists, affected tests, and node context
- A stdio MCP server exposing the existing `codegraph_*` tool names, including `codegraph_explore`
- Workspace discovery through `rootUri`, `workspaceFolders`, and MCP `roots/list`
- Polling watcher auto-sync while the MCP server runs
- Installer and uninstall flows for Claude Code, Cursor, Codex CLI, and opencode
- An experimental `codegraph-ui` Iced workspace member for graph visualization

Remaining TypeScript-only pieces are Liquid and Delphi DFM extractors, Kotlin grammar extraction, tsconfig `extends` chain following, deeper type-analysis edges (`type_of`, `returns`, `instantiates`), and the old TypeScript library API.

<details>
<summary><strong>Manual Setup (Alternative)</strong></summary>

**Install globally:**
```bash
cargo build --release --manifest-path rust/Cargo.toml
mkdir -p ~/.local/bin
cp rust/target/release/codegraph-rs ~/.local/bin/codegraph
```

**Add to `~/.claude.json`:**
```json
{
  "mcpServers": {
    "codegraph": {
      "type": "stdio",
      "command": "codegraph",
      "args": ["serve", "--mcp"]
    }
  }
}
```

**Add to `~/.claude/settings.json` (optional, for auto-allow):**
```json
{
  "permissions": {
    "allow": [
      "mcp__codegraph__codegraph_search",
      "mcp__codegraph__codegraph_context",
      "mcp__codegraph__codegraph_callers",
      "mcp__codegraph__codegraph_callees",
      "mcp__codegraph__codegraph_impact",
      "mcp__codegraph__codegraph_node",
      "mcp__codegraph__codegraph_status",
      "mcp__codegraph__codegraph_files"
    ]
  }
}
```

</details>

<details>
<summary><strong>Global Instructions Reference</strong></summary>

The installer automatically adds these instructions to `~/.claude/CLAUDE.md`:

```markdown
## CodeGraph

CodeGraph builds a semantic knowledge graph of codebases for faster, smarter code exploration.

### If `.codegraph/` exists in the project

**NEVER call `codegraph_explore` or `codegraph_context` directly in the main session.** These tools return large amounts of source code that fills up main session context. Instead, ALWAYS spawn an Explore agent for any exploration question (e.g., "how does X work?", "explain the Y system", "where is Z implemented?").

**When spawning Explore agents**, include this instruction in the prompt:

> This project has CodeGraph initialized (.codegraph/ exists). Use `codegraph_explore` as your PRIMARY tool — it returns full source code sections from all relevant files in one call.
>
> **Rules:**
> 1. Follow the explore call budget in the `codegraph_explore` tool description — it scales automatically based on project size.
> 2. Do NOT re-read files that codegraph_explore already returned source code for. The source sections are complete and authoritative.
> 3. Only fall back to grep/glob/read for files listed under "Additional relevant files" if you need more detail, or if codegraph returned no results.

**The main session may only use these lightweight tools directly** (for targeted lookups before making edits, not for exploration):

| Tool | Use For |
|------|---------|
| `codegraph_search` | Find symbols by name |
| `codegraph_callers` / `codegraph_callees` | Trace call flow |
| `codegraph_impact` | Check what's affected before editing |
| `codegraph_node` | Get a single symbol's details |

### If `.codegraph/` does NOT exist

At the start of a session, ask the user if they'd like to initialize CodeGraph:

"I notice this project doesn't have CodeGraph initialized. Would you like me to run `codegraph init -i` to build a code knowledge graph?"
```

</details>

---

## How It Works

```
┌─────────────────────────────────────────────────────────────────┐
│                        Claude Code                               │
│                                                                  │
│  "Implement user authentication"                                 │
│           │                                                      │
│           ▼                                                      │
│  ┌─────────────────┐      ┌─────────────────┐                   │
│  │  Explore Agent  │ ──── │  Explore Agent  │                   │
│  └────────┬────────┘      └────────┬────────┘                   │
│           │                        │                             │
└───────────┼────────────────────────┼─────────────────────────────┘
            │                        │
            ▼                        ▼
┌───────────────────────────────────────────────────────────────────┐
│                     CodeGraph MCP Server                          │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐               │
│  │   Search    │  │   Callers   │  │   Context   │               │
│  │  "auth"     │  │  "login()"  │  │  for task   │               │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘               │
│         │                │                │                       │
│         └────────────────┼────────────────┘                       │
│                          ▼                                        │
│              ┌───────────────────────┐                            │
│              │   SQLite Graph DB     │                            │
│              │   • 387 symbols       │                            │
│              │   • 1,204 edges       │                            │
│              │   • Instant lookups   │                            │
│              └───────────────────────┘                            │
└───────────────────────────────────────────────────────────────────┘
```

1. **Extraction** — Rust files are parsed with `syn`; most supported languages are parsed with [tree-sitter](https://tree-sitter.github.io/). Svelte and Vue single-file components emit component nodes and parse script blocks for symbols.

2. **Storage** — Everything goes into a local SQLite database (`.codegraph/codegraph.db`) with FTS5 full-text search.

3. **Resolution** — After extraction, CodeGraph resolves imports, tsconfig path aliases, cross-file calls, dependency relationships, and framework-specific route handlers.

4. **Auto-Sync** — The Rust MCP server can run a polling watcher. Changes are debounced, filtered through the project config, and refreshed into the graph while the server is running.

---

## CLI Reference

```bash
codegraph install                 # Run installer
codegraph init [path]             # Initialize a project (-i/--index to also index)
codegraph uninit [path]           # Remove CodeGraph from a project (--force to skip prompt)
codegraph status [path]           # Show statistics (--json)
codegraph scan [path]             # Scan included files without indexing (--json)
codegraph index [path]            # Full refresh index (--json)
codegraph sync [path]             # Refresh the graph from current files
codegraph query <search>          # Search symbols (--kind, --language, --limit)
codegraph files [path]            # List indexed files (--filter, --json)
codegraph context <node-id>       # Show graph context around one node (--json)
codegraph affected [files...]     # Find test files affected by changes (see below)
codegraph unlock [path]           # Remove stale CodeGraph lock files
codegraph serve --mcp             # Start MCP server (--no-watch to disable watcher)
```

### `codegraph affected`

Traces import dependencies transitively to find which test files are affected by changed source files.

```bash
codegraph affected src/utils.ts src/api.ts         # Pass files as arguments
git diff --name-only | codegraph affected --stdin  # Pipe from git diff
codegraph affected src/auth.ts --filter "e2e/*"    # Custom test file pattern
```

| Option | Description | Default |
|--------|-------------|---------|
| `--stdin` | Read file list from stdin | `false` |
| `-d, --depth <n>` | Max dependency traversal depth | `5` |
| `-f, --filter <glob>` | Custom glob to identify test files | auto-detect |
| `-j, --json` | Output as JSON | `false` |
| `-q, --quiet` | Output file paths only | `false` |

**CI/hook example:**

```bash
#!/usr/bin/env bash
AFFECTED=$(git diff --name-only HEAD | codegraph affected --stdin --quiet)
if [ -n "$AFFECTED" ]; then
  npx vitest run $AFFECTED
fi
```

---

## MCP Tools

When running as an MCP server, CodeGraph exposes these tools to Claude Code:

| Tool | Purpose |
|------|---------|
| `codegraph_search` | Find symbols by name across the codebase |
| `codegraph_context` | Build relevant code context for a task |
| `codegraph_callers` | Find what calls a function |
| `codegraph_callees` | Find what a function calls |
| `codegraph_impact` | Analyze what code is affected by changing a symbol |
| `codegraph_explore` | Return source code for related symbols grouped by file, with relationship maps and adaptive budgets |
| `codegraph_node` | Get details about a specific symbol (optionally with source code) |
| `codegraph_files` | Get indexed file structure (faster than filesystem scanning) |
| `codegraph_status` | Check index health and statistics |

---

## Rust Crate Usage

The Rust workspace exposes `codegraph_rs` for local tools that want to query an initialized `.codegraph` database directly.

```rust
use std::path::Path;

use codegraph_rs::{
    config, db, directory, extraction,
    graph::GraphService,
    query::QueryService,
};

fn main() -> anyhow::Result<()> {
    let root = Path::new("/path/to/project");

    if !directory::is_initialized(root) {
        directory::create_directory(root)?;
        let cfg = config::create_default_config(root);
        config::save_config(root, &cfg)?;
        db::initialize_database(root)?;
        extraction::index_project(root, &cfg)?;
    }

    let queries = QueryService::open(root)?;
    let graph = GraphService::new(&queries);
    let results = queries.search_nodes("UserService", None, None, 20)?;

    if let Some(node) = results.first() {
        let callers = graph.get_callers(&node.id, 2)?;
        println!("{} callers", callers.len());
    }

    Ok(())
}
```

---

## Configuration

The `.codegraph/config.json` file controls indexing:

```json
{
  "version": 1,
  "include": ["**/*.ts", "**/*.tsx", "**/*.rs", "**/*.py", "**/*.go"],
  "exclude": ["**/node_modules/**", "**/dist/**", "**/build/**", "**/target/**"],
  "languages": [],
  "frameworks": [],
  "maxFileSize": 1048576,
  "extractDocstrings": true,
  "trackCallSites": true,
  "customPatterns": null
}
```

| Option | Description | Default |
|--------|-------------|---------|
| `include` | Glob patterns to index | common source extensions |
| `exclude` | Glob patterns to ignore | generated/dependency/build dirs |
| `languages` | Languages to index (auto-detected if empty) | `[]` |
| `frameworks` | Framework hints for better resolution | `[]` |
| `maxFileSize` | Skip files larger than this (bytes) | `1048576` (1MB) |
| `extractDocstrings` | Extract docstrings from code | `true` |
| `trackCallSites` | Track call site locations | `true` |
| `customPatterns` | Optional user-defined symbol patterns | `null` |

## Supported Languages

| Language | Extension | Status |
|----------|-----------|--------|
| TypeScript | `.ts`, `.tsx` | Tree-sitter symbol extraction, imports, calls |
| JavaScript | `.js`, `.jsx`, `.mjs` | Tree-sitter symbol extraction, imports, calls |
| Python | `.py` | Tree-sitter symbol extraction, imports, calls |
| Go | `.go` | Tree-sitter symbol extraction, imports, calls |
| Rust | `.rs` | `syn` AST symbol extraction and local calls |
| Java | `.java` | Tree-sitter symbol extraction, imports, calls |
| C# | `.cs` | Tree-sitter symbol extraction, imports, calls |
| PHP | `.php` | Tree-sitter symbol extraction, imports, calls |
| Ruby | `.rb` | Tree-sitter symbol extraction, imports, calls |
| C | `.c`, `.h` | Tree-sitter symbol extraction, imports, calls |
| C++ | `.cpp`, `.hpp`, `.cc`, `.cxx` | Tree-sitter symbol extraction, imports, calls |
| Swift | `.swift` | Tree-sitter symbol extraction, imports, calls |
| Scala | `.scala`, `.sc` | Tree-sitter symbol extraction, imports, calls |
| Dart | `.dart` | Tree-sitter symbol extraction, imports, calls |
| Svelte | `.svelte` | Component node plus script-block extraction |
| Vue | `.vue` | Component node plus script-block extraction |
| Lua | `.lua` | Tree-sitter symbol extraction, imports, calls |
| Luau | `.luau` | Detected and parsed with the Lua extractor |
| Kotlin | `.kt`, `.kts` | File discovery only in Rust; grammar extraction is pending |
| Liquid | `.liquid` | File discovery only in Rust; TypeScript extractor remains the reference |
| Pascal / Delphi | `.pas`, `.dpr`, `.dpk`, `.lpr`, `.dfm`, `.fmx` | File discovery only in Rust; TypeScript extractor remains the reference |

## Troubleshooting

**"CodeGraph not initialized"** — Run `codegraph init -i` in your project directory first.

**Indexing is slow** — Check that generated, dependency, and build directories are excluded in `.codegraph/config.json`. The default config excludes common directories such as `node_modules`, `target`, `dist`, `build`, `.next`, `.venv`, and `.gradle`.

**MCP server not connecting** — Ensure the project is initialized/indexed, verify the binary path in your MCP config, and check that `codegraph serve --mcp` starts from the command line.

**Stale lock file** — Run `codegraph unlock` from the project root.

**Missing symbols** — The MCP server auto-syncs while it is running (wait a couple seconds). Run `codegraph sync` manually if needed. Check that the file's language is symbol-extractable in the Rust table above and is not excluded by config patterns.

## Star History

<a href="https://www.star-history.com/?repos=colbymchenry%2Fcodegraph&type=date&legend=top-left">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=colbymchenry/codegraph&type=date&theme=dark&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=colbymchenry/codegraph&type=date&legend=top-left" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=colbymchenry/codegraph&type=date&legend=top-left" />
 </picture>
</a>

## License

MIT

---

<div align="center">

**Made for AI coding agents — Claude Code, Cursor, Codex CLI, and opencode**

[Report Bug](https://github.com/colbymchenry/codegraph/issues) · [Request Feature](https://github.com/colbymchenry/codegraph/issues)

</div>
