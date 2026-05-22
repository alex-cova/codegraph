use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

const CODEGRAPH_SECTION_START: &str = "<!-- CODEGRAPH_START -->";
const CODEGRAPH_SECTION_END: &str = "<!-- CODEGRAPH_END -->";
const INSTRUCTIONS_TEMPLATE: &str = r#"<!-- CODEGRAPH_START -->
## CodeGraph

This project has a CodeGraph MCP server (`codegraph_*` tools) configured. CodeGraph is a tree-sitter-parsed knowledge graph of every symbol, edge, and file. Reads are sub-millisecond and return structural information grep cannot.

### When to prefer codegraph over native search

Use codegraph for **structural** questions — what calls what, what would break, where is X defined, what is X's signature. Use native grep/read only for **literal text** queries (string contents, comments, log messages) or after you already have a specific file open.

| Question | Tool |
|---|---|
| "Where is X defined?" / "Find symbol named X" | `codegraph_search` |
| "What calls function Y?" | `codegraph_callers` |
| "What does Y call?" | `codegraph_callees` |
| "What would break if I changed Z?" | `codegraph_impact` |
| "Show me Y's signature / source / docstring" | `codegraph_node` |
| "Give me focused context for a task/area" | `codegraph_context` |
| "See several related symbols' source at once" | `codegraph_explore` |
| "What files exist under path/" | `codegraph_files` |
| "Is the index healthy?" | `codegraph_status` |

### Rules of thumb

- **Answer directly — don't delegate exploration.** For "how does X work" / architecture / trace questions, answer with 2-3 codegraph calls: `codegraph_context` first, then ONE `codegraph_explore` for the source of the symbols it surfaces. Codegraph IS the pre-built index, so spawning a separate file-reading sub-task/agent — or running a grep + read loop — repeats work codegraph already did and costs more for the same answer.
- **Trust codegraph results.** They come from a full AST parse. Do NOT re-verify them with grep — that's slower, less accurate, and wastes context.
- **Don't grep first** when looking up a symbol by name. `codegraph_search` is faster and returns kind + location + signature in one call.
- **Don't chain `codegraph_search` + `codegraph_node`** when you just want context — `codegraph_context` is one call.
- **Don't loop `codegraph_node` over many symbols** — one `codegraph_explore` call returns several symbols' source grouped in a single capped call, while each separate node/Read call re-reads the whole context and costs far more.
- **Index lag**: the file watcher debounces ~500ms behind writes; don't re-query immediately after editing a file in the same turn.

### If `.codegraph/` doesn't exist

The MCP server returns "not initialized." Ask the user: *"I notice this project doesn't have CodeGraph initialized. Want me to run `codegraph init -i` to build the index?"*
<!-- CODEGRAPH_END -->"#;

const CURSOR_FRONTMATTER: &str = "---\ndescription: CodeGraph MCP usage guide — when to use which tool\nalwaysApply: true\n---\n\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallLocation {
    Global,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallTarget {
    Claude,
    Cursor,
    Codex,
    Opencode,
}

pub struct InstallOptions {
    pub targets: Vec<InstallTarget>,
    pub location: InstallLocation,
    pub auto_allow: bool,
    pub print_config: Option<InstallTarget>,
    pub uninstall: bool,
}

pub fn run_install(options: InstallOptions) -> Result<()> {
    if let Some(target) = options.print_config {
        print!("{}", print_config(target, options.location)?);
        return Ok(());
    }

    for target in options.targets {
        if options.uninstall {
            uninstall_target(target, options.location)?;
        } else {
            install_target(target, options.location, options.auto_allow)?;
        }
    }

    Ok(())
}

pub fn parse_targets(raw: Option<&str>) -> Result<Vec<InstallTarget>> {
    match raw.unwrap_or("auto") {
        "auto" => Ok(auto_targets()),
        "all" => Ok(vec![
            InstallTarget::Claude,
            InstallTarget::Cursor,
            InstallTarget::Codex,
            InstallTarget::Opencode,
        ]),
        "none" => Ok(Vec::new()),
        raw => raw
            .split(',')
            .map(|item| parse_target_id(item.trim()))
            .collect(),
    }
}

pub fn parse_target_id(raw: &str) -> Result<InstallTarget> {
    match raw {
        "claude" => Ok(InstallTarget::Claude),
        "cursor" => Ok(InstallTarget::Cursor),
        "codex" => Ok(InstallTarget::Codex),
        "opencode" => Ok(InstallTarget::Opencode),
        _ => bail!("unknown install target: {raw}"),
    }
}

pub fn parse_location(raw: Option<&str>, yes: bool) -> Result<InstallLocation> {
    match raw {
        Some("global") => Ok(InstallLocation::Global),
        Some("local") => Ok(InstallLocation::Local),
        Some(other) => bail!("--location must be \"global\" or \"local\" (got \"{other}\")"),
        None if yes => Ok(InstallLocation::Global),
        None => Ok(InstallLocation::Global),
    }
}

fn auto_targets() -> Vec<InstallTarget> {
    let home = home_dir();
    let mut targets = Vec::new();
    if home.join(".claude").exists() || home.join(".claude.json").exists() {
        targets.push(InstallTarget::Claude);
    }
    if home.join(".cursor").exists() {
        targets.push(InstallTarget::Cursor);
    }
    if home.join(".codex").exists() {
        targets.push(InstallTarget::Codex);
    }
    if opencode_global_dir().exists() {
        targets.push(InstallTarget::Opencode);
    }
    if targets.is_empty() {
        targets.push(InstallTarget::Codex);
    }
    targets
}

fn install_target(target: InstallTarget, location: InstallLocation, auto_allow: bool) -> Result<()> {
    match target {
        InstallTarget::Claude => install_claude(location, auto_allow),
        InstallTarget::Cursor => install_cursor(location),
        InstallTarget::Codex => {
            if location == InstallLocation::Local {
                bail!("Codex CLI has no project-local config; re-run with --location=global");
            }
            install_codex()
        }
        InstallTarget::Opencode => install_opencode(location),
    }
}

fn uninstall_target(target: InstallTarget, location: InstallLocation) -> Result<()> {
    match target {
        InstallTarget::Claude => uninstall_claude(location),
        InstallTarget::Cursor => uninstall_cursor(location),
        InstallTarget::Codex => {
            if location == InstallLocation::Local {
                return Ok(());
            }
            uninstall_codex()
        }
        InstallTarget::Opencode => uninstall_opencode(location),
    }
}

fn print_config(target: InstallTarget, location: InstallLocation) -> Result<String> {
    Ok(match target {
        InstallTarget::Claude => {
            let target_path = if location == InstallLocation::Global {
                home_dir().join(".claude.json")
            } else {
                cwd().join(".mcp.json")
            };
            format!(
                "# Add to {}\n\n{}\n",
                target_path.display(),
                serde_json::to_string_pretty(&json!({
                    "mcpServers": {
                        "codegraph": base_mcp_server_config(None)
                    }
                }))?
            )
        }
        InstallTarget::Cursor => {
            let target_path = if location == InstallLocation::Global {
                home_dir().join(".cursor").join("mcp.json")
            } else {
                cwd().join(".cursor").join("mcp.json")
            };
            format!(
                "# Add to {}\n\n{}\n",
                target_path.display(),
                serde_json::to_string_pretty(&json!({
                    "mcpServers": {
                        "codegraph": cursor_mcp_server_config(location)
                    }
                }))?
            )
        }
        InstallTarget::Codex => {
            let target_path = home_dir().join(".codex").join("config.toml");
            format!(
                "# Add to {}\n\n{}\n",
                target_path.display(),
                build_toml_table(
                    "mcp_servers.codegraph",
                    "codegraph",
                    &["serve", "--mcp"]
                )
            )
        }
        InstallTarget::Opencode => {
            let target_path = opencode_config_path(location);
            format!(
                "# Add to {}\n\n{}\n",
                target_path.display(),
                serde_json::to_string_pretty(&json!({
                    "$schema": "https://opencode.ai/config.json",
                    "mcp": {
                        "codegraph": {
                            "type": "local",
                            "command": ["codegraph", "serve", "--mcp"],
                            "enabled": true
                        }
                    }
                }))?
            )
        }
    })
}

fn install_claude(location: InstallLocation, auto_allow: bool) -> Result<()> {
    let mcp_path = if location == InstallLocation::Global {
        home_dir().join(".claude.json")
    } else {
        cwd().join(".mcp.json")
    };
    let mut config = read_json_object(&mcp_path)?;
    set_nested_object(
        &mut config,
        &["mcpServers", "codegraph"],
        json!(base_mcp_server_config(None)),
    );
    write_json_file(&mcp_path, &config)?;

    if auto_allow {
        let settings_path = if location == InstallLocation::Global {
            home_dir().join(".claude").join("settings.json")
        } else {
            cwd().join(".claude").join("settings.json")
        };
        let mut settings = read_json_object(&settings_path)?;
        let allow = settings
            .pointer_mut("/permissions/allow")
            .and_then(Value::as_array_mut);
        if let Some(allow) = allow {
            merge_permissions(allow);
        } else {
            set_nested_object(
                &mut settings,
                &["permissions", "allow"],
                json!(claude_permissions()),
            );
        }
        write_json_file(&settings_path, &settings)?;
    }

    let instructions_path = if location == InstallLocation::Global {
        home_dir().join(".claude").join("CLAUDE.md")
    } else {
        cwd().join(".claude").join("CLAUDE.md")
    };
    replace_or_append_marked_section(&instructions_path, INSTRUCTIONS_TEMPLATE)?;
    Ok(())
}

fn install_cursor(location: InstallLocation) -> Result<()> {
    let mcp_path = if location == InstallLocation::Global {
        home_dir().join(".cursor").join("mcp.json")
    } else {
        cwd().join(".cursor").join("mcp.json")
    };
    let mut config = read_json_object(&mcp_path)?;
    set_nested_object(
        &mut config,
        &["mcpServers", "codegraph"],
        json!(cursor_mcp_server_config(location)),
    );
    write_json_file(&mcp_path, &config)?;

    if location == InstallLocation::Local {
        let rules_path = cwd().join(".cursor").join("rules").join("codegraph.mdc");
        ensure_parent_dir(&rules_path)?;
        if !rules_path.exists() {
            fs::write(&rules_path, format!("{CURSOR_FRONTMATTER}{INSTRUCTIONS_TEMPLATE}\n"))?;
        } else {
            replace_or_append_marked_section(&rules_path, INSTRUCTIONS_TEMPLATE)?;
        }
    }
    Ok(())
}

fn install_codex() -> Result<()> {
    let config_path = home_dir().join(".codex").join("config.toml");
    ensure_parent_dir(&config_path)?;
    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    let block = build_toml_table("mcp_servers.codegraph", "codegraph", &["serve", "--mcp"]);
    let updated = upsert_toml_table(&existing, "mcp_servers.codegraph", &block);
    fs::write(&config_path, updated)?;

    let instructions_path = home_dir().join(".codex").join("AGENTS.md");
    replace_or_append_marked_section(&instructions_path, INSTRUCTIONS_TEMPLATE)?;
    Ok(())
}

fn install_opencode(location: InstallLocation) -> Result<()> {
    let config_path = opencode_config_path(location);
    let mut config = read_json_object(&config_path)?;
    if config.is_null() || !config.is_object() {
        config = json!({});
    }
    if config.get("$schema").is_none() {
        config["$schema"] = json!("https://opencode.ai/config.json");
    }
    set_nested_object(
        &mut config,
        &["mcp", "codegraph"],
        json!({
            "type": "local",
            "command": ["codegraph", "serve", "--mcp"],
            "enabled": true
        }),
    );
    write_json_file(&config_path, &config)?;

    let instructions_path = if location == InstallLocation::Global {
        opencode_global_dir().join("AGENTS.md")
    } else {
        cwd().join("AGENTS.md")
    };
    replace_or_append_marked_section(&instructions_path, INSTRUCTIONS_TEMPLATE)?;
    Ok(())
}

fn uninstall_claude(location: InstallLocation) -> Result<()> {
    let mcp_path = if location == InstallLocation::Global {
        home_dir().join(".claude.json")
    } else {
        cwd().join(".mcp.json")
    };
    remove_nested_key(&mcp_path, &["mcpServers", "codegraph"])?;

    let settings_path = if location == InstallLocation::Global {
        home_dir().join(".claude").join("settings.json")
    } else {
        cwd().join(".claude").join("settings.json")
    };
    remove_claude_permissions(&settings_path)?;

    let instructions_path = if location == InstallLocation::Global {
        home_dir().join(".claude").join("CLAUDE.md")
    } else {
        cwd().join(".claude").join("CLAUDE.md")
    };
    remove_marked_section(&instructions_path)?;
    Ok(())
}

fn uninstall_cursor(location: InstallLocation) -> Result<()> {
    let mcp_path = if location == InstallLocation::Global {
        home_dir().join(".cursor").join("mcp.json")
    } else {
        cwd().join(".cursor").join("mcp.json")
    };
    remove_nested_key(&mcp_path, &["mcpServers", "codegraph"])?;
    if location == InstallLocation::Local {
        remove_marked_section(&cwd().join(".cursor").join("rules").join("codegraph.mdc"))?;
    }
    Ok(())
}

fn uninstall_codex() -> Result<()> {
    let config_path = home_dir().join(".codex").join("config.toml");
    if config_path.exists() {
        let existing = fs::read_to_string(&config_path).unwrap_or_default();
        let updated = remove_toml_table(&existing, "mcp_servers.codegraph");
        if updated.trim().is_empty() {
            let _ = fs::remove_file(&config_path);
        } else {
            fs::write(&config_path, normalize_trailing_newline(updated))
                .with_context(|| format!("failed to write {}", config_path.display()))?;
        }
    }
    remove_marked_section(&home_dir().join(".codex").join("AGENTS.md"))?;
    Ok(())
}

fn uninstall_opencode(location: InstallLocation) -> Result<()> {
    let config_path = opencode_config_path(location);
    remove_nested_key(&config_path, &["mcp", "codegraph"])?;
    remove_marked_section(&if location == InstallLocation::Global {
        opencode_global_dir().join("AGENTS.md")
    } else {
        cwd().join("AGENTS.md")
    })?;
    Ok(())
}

fn claude_permissions() -> Vec<&'static str> {
    vec![
        "mcp__codegraph__codegraph_search",
        "mcp__codegraph__codegraph_context",
        "mcp__codegraph__codegraph_callers",
        "mcp__codegraph__codegraph_callees",
        "mcp__codegraph__codegraph_impact",
        "mcp__codegraph__codegraph_node",
        "mcp__codegraph__codegraph_status",
    ]
}

fn merge_permissions(allow: &mut Vec<Value>) {
    for permission in claude_permissions() {
        if !allow.iter().any(|value| value.as_str() == Some(permission)) {
            allow.push(json!(permission));
        }
    }
}

fn base_mcp_server_config(path: Option<String>) -> serde_json::Map<String, Value> {
    let mut config = serde_json::Map::from_iter([
        ("type".to_string(), json!("stdio")),
        ("command".to_string(), json!("codegraph")),
        ("args".to_string(), json!(["serve", "--mcp"])),
    ]);
    if let Some(path) = path {
        config.insert("args".to_string(), json!(["serve", "--mcp", "--path", path]));
    }
    config
}

fn cursor_mcp_server_config(location: InstallLocation) -> serde_json::Map<String, Value> {
    let path_arg = if location == InstallLocation::Global {
        "${workspaceFolder}".to_string()
    } else {
        cwd().display().to_string()
    };
    base_mcp_server_config(Some(path_arg))
}

fn read_json_object(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&content).or_else(|_| Ok(json!({})))
}

fn write_json_file(path: &Path, value: &Value) -> Result<()> {
    ensure_parent_dir(path)?;
    fs::write(path, serde_json::to_string_pretty(value)? + "\n")
        .with_context(|| format!("failed to write {}", path.display()))
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    Ok(())
}

fn set_nested_object(root: &mut Value, path: &[&str], value: Value) {
    if path.is_empty() {
        *root = value;
        return;
    }
    if !root.is_object() {
        *root = json!({});
    }
    let mut current = root;
    for key in &path[..path.len() - 1] {
        if !current[*key].is_object() {
            current[*key] = json!({});
        }
        current = &mut current[*key];
    }
    current[path[path.len() - 1]] = value;
}

fn remove_nested_key(path: &Path, key_path: &[&str]) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut value = read_json_object(path)?;
    remove_nested_key_from_value(&mut value, key_path);
    if is_effectively_empty_object(&value) {
        let _ = fs::remove_file(path);
        return Ok(());
    }
    write_json_file(path, &value)
}

fn remove_nested_key_from_value(value: &mut Value, key_path: &[&str]) {
    if key_path.is_empty() || !value.is_object() {
        return;
    }
    if key_path.len() == 1 {
        if let Some(object) = value.as_object_mut() {
            object.remove(key_path[0]);
        }
        return;
    }
    if let Some(child) = value.get_mut(key_path[0]) {
        remove_nested_key_from_value(child, &key_path[1..]);
        let should_prune = child.as_object().map(|obj| obj.is_empty()).unwrap_or(false);
        if should_prune {
            if let Some(object) = value.as_object_mut() {
                object.remove(key_path[0]);
            }
        }
    }
}

fn remove_claude_permissions(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut settings = read_json_object(path)?;
    if let Some(allow) = settings.pointer_mut("/permissions/allow").and_then(Value::as_array_mut) {
        allow.retain(|value| {
            value
                .as_str()
                .map(|item| !item.starts_with("mcp__codegraph__"))
                .unwrap_or(true)
        });
    }
    prune_empty_containers(&mut settings);
    if is_effectively_empty_object(&settings) {
        let _ = fs::remove_file(path);
        return Ok(());
    }
    write_json_file(path, &settings)
}

fn replace_or_append_marked_section(path: &Path, body: &str) -> Result<()> {
    ensure_parent_dir(path)?;
    if !path.exists() {
        fs::write(path, format!("{body}\n"))
            .with_context(|| format!("failed to write {}", path.display()))?;
        return Ok(());
    }

    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let updated = if let (Some(start), Some(end)) = (
        content.find(CODEGRAPH_SECTION_START),
        content.find(CODEGRAPH_SECTION_END),
    ) {
        if end > start {
            let end_idx = end + CODEGRAPH_SECTION_END.len();
            format!("{}{}{}", &content[..start], body, &content[end_idx..])
        } else {
            append_with_spacing(&content, body)
        }
    } else {
        append_with_spacing(&content, body)
    };

    fs::write(path, normalize_trailing_newline(updated))
        .with_context(|| format!("failed to write {}", path.display()))
}

fn remove_marked_section(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let updated = if let (Some(start), Some(end)) = (
        content.find(CODEGRAPH_SECTION_START),
        content.find(CODEGRAPH_SECTION_END),
    ) {
        if end > start {
            let end_idx = end + CODEGRAPH_SECTION_END.len();
            let before = content[..start].trim_end();
            let after = content[end_idx..].trim_start();
            if before.is_empty() && after.is_empty() {
                String::new()
            } else if before.is_empty() {
                after.to_string()
            } else if after.is_empty() {
                before.to_string()
            } else {
                format!("{before}\n\n{after}")
            }
        } else {
            content
        }
    } else {
        content
    };
    if updated.trim().is_empty() {
        let _ = fs::remove_file(path);
        return Ok(());
    }
    fs::write(path, normalize_trailing_newline(updated))
        .with_context(|| format!("failed to write {}", path.display()))
}

fn append_with_spacing(content: &str, body: &str) -> String {
    let trimmed = content.trim_end();
    let sep = if trimmed.is_empty() { "" } else { "\n\n" };
    format!("{trimmed}{sep}{body}")
}

fn normalize_trailing_newline(content: String) -> String {
    let trimmed = content.trim_end();
    format!("{trimmed}\n")
}

fn build_toml_table(header: &str, command: &str, args: &[&str]) -> String {
    let args = args
        .iter()
        .map(|arg| format!("\"{}\"", arg.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{header}]\ncommand = \"{command}\"\nargs = [{args}]")
}

fn upsert_toml_table(existing: &str, header: &str, block: &str) -> String {
    let header_line = format!("[{header}]");
    if let Some(start) = find_header(existing, &header_line) {
        let end = find_next_header(existing, start + header_line.len());
        let before = existing[..start].trim_end();
        let after = existing[end..].trim_start();
        let sep_before = if before.is_empty() { "" } else { "\n\n" };
        let sep_after = if after.is_empty() { "\n" } else { "\n\n" };
        return format!("{before}{sep_before}{block}{sep_after}{after}");
    }

    let trimmed = existing.trim_end();
    let sep = if trimmed.is_empty() { "" } else { "\n\n" };
    format!("{trimmed}{sep}{block}\n")
}

fn remove_toml_table(existing: &str, header: &str) -> String {
    let header_line = format!("[{header}]");
    let Some(start) = find_header(existing, &header_line) else {
        return existing.to_string();
    };
    let end = find_next_header(existing, start + header_line.len());
    let before = existing[..start].trim_end();
    let after = existing[end..].trim_start();
    if before.is_empty() && after.is_empty() {
        String::new()
    } else if before.is_empty() {
        after.to_string()
    } else if after.is_empty() {
        before.to_string()
    } else {
        format!("{before}\n\n{after}")
    }
}

fn find_header(content: &str, header_line: &str) -> Option<usize> {
    if content.starts_with(header_line) {
        return Some(0);
    }
    content.find(&format!("\n{header_line}")).map(|idx| idx + 1)
}

fn find_next_header(content: &str, from: usize) -> usize {
    let bytes = content.as_bytes();
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\n' && bytes[i + 1] == b'[' && bytes.get(i + 2) != Some(&b'[') {
            return i + 1;
        }
        i += 1;
    }
    content.len()
}

fn opencode_global_dir() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join("AppData").join("Roaming"))
            .join("opencode")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join(".config"))
            .join("opencode")
    }
}

fn opencode_config_path(location: InstallLocation) -> PathBuf {
    if location == InstallLocation::Global {
        opencode_global_dir().join("opencode.jsonc")
    } else {
        cwd().join("opencode.jsonc")
    }
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd())
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn prune_empty_containers(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let keys = map.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                if let Some(child) = map.get_mut(&key) {
                    prune_empty_containers(child);
                }
                let remove = map
                    .get(&key)
                    .map(|child| matches!(child, Value::Object(obj) if obj.is_empty() || matches!(obj.get("allow"), Some(Value::Array(items)) if items.is_empty())))
                    .unwrap_or(false);
                if remove {
                    map.remove(&key);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                prune_empty_containers(item);
            }
        }
        _ => {}
    }
}

fn is_effectively_empty_object(value: &Value) -> bool {
    matches!(value, Value::Object(map) if map.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_targets_supports_all_and_csv() {
        let all = parse_targets(Some("all")).unwrap();
        assert_eq!(all.len(), 4);

        let subset = parse_targets(Some("claude,codex")).unwrap();
        assert_eq!(subset, vec![InstallTarget::Claude, InstallTarget::Codex]);
    }

    #[test]
    fn remove_toml_table_strips_only_target_block() {
        let input = "[foo]\na = 1\n\n[mcp_servers.codegraph]\ncommand = \"codegraph\"\nargs = [\"serve\", \"--mcp\"]\n\n[bar]\nb = 2\n";
        let output = remove_toml_table(input, "mcp_servers.codegraph");
        assert!(output.contains("[foo]"));
        assert!(output.contains("[bar]"));
        assert!(!output.contains("[mcp_servers.codegraph]"));
    }

    #[test]
    fn remove_nested_key_prunes_empty_parents() {
        let mut value = json!({
            "mcpServers": {
                "codegraph": {
                    "command": "codegraph"
                }
            }
        });
        remove_nested_key_from_value(&mut value, &["mcpServers", "codegraph"]);
        assert_eq!(value, json!({}));
    }
}
