use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use quote::ToTokens;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use syn::spanned::Spanned;
use syn::visit::Visit;

use crate::db;
use crate::types::CodeGraphConfig;

const CODEGRAPH_IGNORE_MARKER: &str = ".codegraphignore";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub file_count: usize,
    pub files_by_language: BTreeMap<String, usize>,
    pub files: Vec<ScannedFile>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedFile {
    pub path: String,
    pub language: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexSummary {
    pub files_indexed: usize,
    pub rust_files_indexed: usize,
    pub nodes_created: usize,
    pub edges_created: usize,
    pub files_by_language: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSummary {
    pub files_checked: usize,
    pub files_reindexed: usize,
    pub rust_files_indexed: usize,
    pub nodes_updated: usize,
    pub edges_updated: usize,
}

pub fn detect_language(file_path: &str, source: Option<&str>) -> String {
    let ext = Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| format!(".{}", ext.to_lowercase()))
        .unwrap_or_default();

    let mut language = match ext.as_str() {
        ".ts" => "typescript",
        ".js" => "javascript",
        ".tsx" => "tsx",
        ".jsx" => "jsx",
        ".py" => "python",
        ".go" => "go",
        ".rs" => "rust",
        ".java" => "java",
        ".c" | ".h" => "c",
        ".cpp" | ".hpp" | ".cc" | ".cxx" => "cpp",
        ".cs" => "csharp",
        ".php" => "php",
        ".rb" => "ruby",
        ".swift" => "swift",
        ".kt" | ".kts" => "kotlin",
        ".dart" => "dart",
        ".svelte" => "svelte",
        ".vue" => "vue",
        ".liquid" => "liquid",
        ".pas" | ".dpr" | ".dpk" | ".lpr" | ".dfm" | ".fmx" => "pascal",
        ".scala" | ".sc" => "scala",
        ".lua" => "lua",
        ".luau" => "luau",
        _ => "unknown",
    }
    .to_string();

    if language == "c" && ext == ".h" {
        if let Some(source) = source {
            if looks_like_cpp(source) {
                language = "cpp".to_string();
            }
        }
    }

    language
}

pub fn should_include_file(file_path: &str, config: &CodeGraphConfig) -> Result<bool> {
    let normalized = normalize_path(file_path);
    let exclude = build_globset(&config.exclude)?;
    if exclude.is_match(&normalized) {
        return Ok(false);
    }

    let include = build_globset(&config.include)?;
    Ok(include.is_match(&normalized))
}

pub fn scan_directory(root_dir: &Path, config: &CodeGraphConfig) -> Result<Vec<String>> {
    if let Some(files) = get_git_visible_files(root_dir)? {
        let mut included = Vec::new();
        for file in files {
            if should_include_file(&file, config)? {
                included.push(file);
            }
        }
        return Ok(included);
    }

    scan_directory_walk(root_dir, config)
}

pub fn scan_summary(root_dir: &Path, config: &CodeGraphConfig) -> Result<ScanSummary> {
    let mut files = scan_directory(root_dir, config)?;
    files.sort();

    let mut files_by_language = BTreeMap::new();
    let mut scanned = Vec::new();
    for file in files {
        let language = detect_language_from_disk(root_dir, &file)?;
        *files_by_language.entry(language.clone()).or_insert(0) += 1;
        scanned.push(ScannedFile { path: file, language });
    }

    Ok(ScanSummary {
        file_count: scanned.len(),
        files_by_language,
        files: scanned,
    })
}

pub fn index_project(root_dir: &Path, config: &CodeGraphConfig) -> Result<IndexSummary> {
    let summary = scan_summary(root_dir, config)?;
    let conn = db::open_connection(root_dir)?;
    conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
        .context("failed to begin transaction")?;

    let mut rust_files_indexed = 0usize;
    let mut nodes_created = 0usize;
    let mut edges_created = 0usize;
    let mut all_unresolved_calls: Vec<(String, String, Option<i64>)> = Vec::new();

    let result = (|| -> Result<()> {
        for file in &summary.files {
            let full_path = root_dir.join(&file.path);
            let content = fs::read_to_string(&full_path)
                .with_context(|| format!("failed to read {}", full_path.display()))?;
            let metadata = fs::metadata(&full_path)
                .with_context(|| format!("failed to stat {}", full_path.display()))?;

            clear_file_index(&conn, &file.path)?;
            let file_node = FileNode::new(&file.path, &file.language, &content);
            let mut file_node_count = 1i64;

            if is_symbol_extractable_language(&file.language) {
                if file.language == "rust" {
                    rust_files_indexed += 1;
                }
                let artifacts = extract_language_artifacts(&file.path, &file.language, &content);
                file_node_count += artifacts.symbols.len() as i64;
                insert_file_record(&conn, &file.path, &file.language, &content, &metadata, file_node_count)?;
                insert_file_node(&conn, &file_node)?;
                nodes_created += 1;
                for symbol in artifacts.symbols {
                    insert_symbol(&conn, &symbol)?;
                    let parent_id = symbol.parent_id.as_deref().unwrap_or(&file_node.id);
                    insert_contains_edge(&conn, parent_id, &symbol.id)?;
                    nodes_created += 1;
                    edges_created += 1;
                }
                for edge in artifacts.edges {
                    insert_graph_edge(&conn, &edge)?;
                    edges_created += 1;
                }
                all_unresolved_calls.extend(artifacts.unresolved_calls);
            } else {
                insert_file_record(&conn, &file.path, &file.language, &content, &metadata, file_node_count)?;
                insert_file_node(&conn, &file_node)?;
                nodes_created += 1;
            }
        }
        Ok(())
    })();

    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT;")
                .context("failed to commit transaction")?;
        }
        Err(err) => {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(err);
        }
    }

    // Post-indexing resolution pass: imports → cross-file calls → framework routes
    edges_created += crate::resolution::resolve_imports(&conn, root_dir)
        .unwrap_or(0);
    edges_created += crate::resolution::resolve_cross_file_calls(&conn, &all_unresolved_calls)
        .unwrap_or(0);
    let (route_nodes, route_edges) =
        crate::frameworks::extract_framework_routes(&conn, root_dir).unwrap_or((0, 0));
    nodes_created += route_nodes;
    edges_created += route_edges;

    Ok(IndexSummary {
        files_indexed: summary.file_count,
        rust_files_indexed,
        nodes_created,
        edges_created,
        files_by_language: summary.files_by_language,
    })
}

pub fn sync_project(root_dir: &Path, config: &CodeGraphConfig) -> Result<SyncSummary> {
    let summary = index_project(root_dir, config)?;
    Ok(SyncSummary {
        files_checked: summary.files_indexed,
        files_reindexed: summary.files_indexed,
        rust_files_indexed: summary.rust_files_indexed,
        nodes_updated: summary.nodes_created,
        edges_updated: summary.edges_created,
    })
}

fn detect_language_from_disk(root_dir: &Path, relative_path: &str) -> Result<String> {
    let full_path = root_dir.join(relative_path);
    let needs_header_probe = relative_path.ends_with(".h");
    let source = if needs_header_probe {
        Some(
            fs::read_to_string(&full_path)
                .with_context(|| format!("failed to read {}", full_path.display()))?,
        )
    } else {
        None
    };
    Ok(detect_language(relative_path, source.as_deref()))
}

fn scan_directory_walk(root_dir: &Path, config: &CodeGraphConfig) -> Result<Vec<String>> {
    let mut files = Vec::new();
    let mut visited_dirs = HashSet::new();
    walk_dir(root_dir, root_dir, config, &mut visited_dirs, &mut files)?;
    Ok(files)
}

fn walk_dir(
    root_dir: &Path,
    current_dir: &Path,
    config: &CodeGraphConfig,
    visited_dirs: &mut HashSet<PathBuf>,
    files: &mut Vec<String>,
) -> Result<()> {
    let real_dir = match fs::canonicalize(current_dir) {
        Ok(path) => path,
        Err(_) => return Ok(()),
    };
    if !visited_dirs.insert(real_dir) {
        return Ok(());
    }

    if current_dir.join(CODEGRAPH_IGNORE_MARKER).exists() {
        return Ok(());
    }

    let entries = match fs::read_dir(current_dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let full_path = entry.path();
        let file_type = match entry.file_type() {
            Ok(kind) => kind,
            Err(_) => continue,
        };
        let relative_path = normalize_path(
            &full_path
                .strip_prefix(root_dir)
                .unwrap_or(&full_path)
                .to_string_lossy(),
        );

        if file_type.is_symlink() {
            let target = match fs::canonicalize(&full_path) {
                Ok(path) => path,
                Err(_) => continue,
            };
            let metadata = match fs::metadata(&target) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if metadata.is_dir() {
                if !is_excluded_dir(config, &relative_path)? {
                    walk_dir(root_dir, &full_path, config, visited_dirs, files)?;
                }
            } else if metadata.is_file() && should_include_file(&relative_path, config)? {
                files.push(relative_path);
            }
            continue;
        }

        if file_type.is_dir() {
            if !is_excluded_dir(config, &relative_path)? {
                walk_dir(root_dir, &full_path, config, visited_dirs, files)?;
            }
        } else if file_type.is_file() && should_include_file(&relative_path, config)? {
            files.push(relative_path);
        }
    }

    Ok(())
}

fn is_excluded_dir(config: &CodeGraphConfig, relative_path: &str) -> Result<bool> {
    let exclude = build_globset(&config.exclude)?;
    let dir_pattern = format!("{}/", relative_path.trim_end_matches('/'));
    Ok(exclude.is_match(relative_path) || exclude.is_match(&dir_pattern))
}

fn get_git_visible_files(root_dir: &Path) -> Result<Option<BTreeSet<String>>> {
    let git_root = match command_output(root_dir, &["git", "rev-parse", "--show-toplevel"]) {
        Ok(output) => output.trim().to_string(),
        Err(_) => return Ok(None),
    };

    if fs::canonicalize(&git_root).ok().as_deref() != fs::canonicalize(root_dir).ok().as_deref() {
        let root_abs = root_dir
            .canonicalize()
            .unwrap_or_else(|_| root_dir.to_path_buf())
            .to_string_lossy()
            .to_string();
        if command_status(root_dir, &["git", "check-ignore", "-q", &root_abs]).unwrap_or(false) {
            return Ok(None);
        }
    }

    let mut files = BTreeSet::new();
    collect_git_files(root_dir, "", &mut files)?;
    Ok(Some(files))
}

fn collect_git_files(root_dir: &Path, prefix: &str, files: &mut BTreeSet<String>) -> Result<()> {
    let tracked = command_output(root_dir, &["git", "ls-files", "-c", "--recurse-submodules"])?;
    for line in tracked.lines().map(str::trim).filter(|line| !line.is_empty()) {
        files.insert(normalize_path(&format!("{prefix}{line}")));
    }

    let untracked = command_output(root_dir, &["git", "ls-files", "-o", "--exclude-standard"])?;
    for line in untracked.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(dir) = line.strip_suffix('/') {
            let child_dir = root_dir.join(dir);
            if child_dir.join(".git").exists() {
                collect_git_files(&child_dir, &format!("{prefix}{dir}/"), files)?;
            }
            continue;
        }
        files.insert(normalize_path(&format!("{prefix}{line}")));
    }

    Ok(())
}

fn command_output(root_dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new(args[0])
        .args(&args[1..])
        .current_dir(root_dir)
        .output()
        .with_context(|| format!("failed to run {}", args.join(" ")))?;
    if !output.status.success() {
        anyhow::bail!("command failed: {}", args.join(" "));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn command_status(root_dir: &Path, args: &[&str]) -> Result<bool> {
    let status = Command::new(args[0])
        .args(&args[1..])
        .current_dir(root_dir)
        .status()
        .with_context(|| format!("failed to run {}", args.join(" ")))?;
    Ok(status.success())
}

fn build_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(
            Glob::new(pattern)
                .with_context(|| format!("invalid glob pattern: {pattern}"))?,
        );
    }
    builder.build().context("failed to build glob matcher")
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn looks_like_cpp(source: &str) -> bool {
    let sample: String = source.chars().take(8192).collect();
    regex::Regex::new(
        r"\bnamespace\b|\bclass\s+\w+\s*[:{]|\btemplate\s*<|\b(?:public|private|protected)\s*:|\bvirtual\b|\busing\s+(?:namespace\b|\w+\s*=)",
    )
    .map(|regex| regex.is_match(&sample))
    .unwrap_or(false)
}

#[derive(Debug, Clone)]
struct FileNode {
    id: String,
    name: String,
    qualified_name: String,
    file_path: String,
    language: String,
    end_line: i64,
}

impl FileNode {
    fn new(file_path: &str, language: &str, content: &str) -> Self {
        Self {
            id: format!("file:{file_path}"),
            name: Path::new(file_path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(file_path)
                .to_string(),
            qualified_name: file_path.to_string(),
            file_path: file_path.to_string(),
            language: language.to_string(),
            end_line: content.lines().count().max(1) as i64,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RustSymbol {
    pub(crate) id: String,
    pub(crate) kind: &'static str,
    pub(crate) name: String,
    pub(crate) qualified_name: String,
    pub(crate) parent_id: Option<String>,
    pub(crate) file_path: String,
    pub(crate) language: &'static str,
    pub(crate) start_line: i64,
    pub(crate) end_line: i64,
    pub(crate) signature: Option<String>,
    pub(crate) visibility: Option<&'static str>,
    pub(crate) is_exported: bool,
    pub(crate) is_async: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct RustGraphEdge {
    pub(crate) source: String,
    pub(crate) target: String,
    pub(crate) kind: &'static str,
    pub(crate) line: Option<i64>,
}

fn clear_file_index(conn: &Connection, file_path: &str) -> Result<()> {
    conn.execute("DELETE FROM files WHERE path = ?1", [file_path])?;
    conn.execute("DELETE FROM nodes WHERE file_path = ?1", [file_path])?;
    Ok(())
}

fn insert_file_record(
    conn: &Connection,
    file_path: &str,
    language: &str,
    content: &str,
    metadata: &fs::Metadata,
    node_count: i64,
) -> Result<()> {
    let content_hash = hash_content(content);
    let modified_at = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_else(unix_time_ms);
    let indexed_at = unix_time_ms();
    conn.execute(
        "INSERT INTO files (path, content_hash, language, size, modified_at, indexed_at, node_count, errors)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
        params![
            file_path,
            content_hash,
            language,
            i64::try_from(metadata.len()).unwrap_or(i64::MAX),
            modified_at,
            indexed_at,
            node_count
        ],
    )?;
    Ok(())
}

fn insert_file_node(conn: &Connection, node: &FileNode) -> Result<()> {
    conn.execute(
        "INSERT INTO nodes (
            id, kind, name, qualified_name, file_path, language,
            start_line, end_line, start_column, end_column, docstring, signature, visibility,
            is_exported, is_async, is_static, is_abstract, decorators, type_parameters, updated_at
         ) VALUES (
            ?1, 'file', ?2, ?3, ?4, ?5,
            1, ?6, 0, 0, NULL, NULL, NULL,
            0, 0, 0, 0, NULL, NULL, ?7
         )",
        params![
            node.id,
            node.name,
            node.qualified_name,
            node.file_path,
            node.language,
            node.end_line,
            unix_time_ms()
        ],
    )?;
    Ok(())
}

fn insert_symbol(conn: &Connection, symbol: &RustSymbol) -> Result<()> {
    conn.execute(
        "INSERT INTO nodes (
            id, kind, name, qualified_name, file_path, language,
            start_line, end_line, start_column, end_column, docstring, signature, visibility,
            is_exported, is_async, is_static, is_abstract, decorators, type_parameters, updated_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, 0, 0, NULL, ?9, ?10,
            ?11, ?12, 0, 0, NULL, NULL, ?13
         )",
        params![
            symbol.id,
            symbol.kind,
            symbol.name,
            symbol.qualified_name,
            symbol.file_path,
            symbol.language,
            symbol.start_line,
            symbol.end_line,
            symbol.signature,
            symbol.visibility,
            if symbol.is_exported { 1 } else { 0 },
            if symbol.is_async { 1 } else { 0 },
            unix_time_ms()
        ],
    )?;
    Ok(())
}

fn insert_contains_edge(conn: &Connection, source: &str, target: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO edges (source, target, kind, metadata, line, col, provenance)
         VALUES (?1, ?2, 'contains', NULL, NULL, NULL, 'heuristic')",
        params![source, target],
    )?;
    Ok(())
}

fn insert_graph_edge(conn: &Connection, edge: &RustGraphEdge) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO edges (source, target, kind, metadata, line, col, provenance)
         VALUES (?1, ?2, ?3, NULL, ?4, NULL, 'heuristic')",
        params![edge.source, edge.target, edge.kind, edge.line],
    )?;
    Ok(())
}

fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn is_symbol_extractable_language(language: &str) -> bool {
    matches!(
        language,
        "rust"
            | "typescript"
            | "javascript"
            | "tsx"
            | "jsx"
            | "python"
            | "go"
            | "java"
            | "c"
            | "cpp"
            | "csharp"
            | "ruby"
            | "php"
            | "swift"
            | "lua"
            | "dart"
            | "scala"
            | "svelte"
            | "vue"
    )
}

fn extract_language_artifacts(file_path: &str, language: &str, content: &str) -> RustArtifacts {
    match language {
        "rust" => extract_rust_artifacts(file_path, content),
        "svelte" => crate::svelte_vue::extract_svelte(file_path, content),
        "vue" => crate::svelte_vue::extract_vue(file_path, content),
        _ => crate::ts_extraction::extract_with_tree_sitter(file_path, language, content)
            .unwrap_or_default(),
    }
}

fn extract_rust_artifacts(file_path: &str, content: &str) -> RustArtifacts {
    let parsed = match syn::parse_file(content) {
        Ok(file) => file,
        Err(_) => return RustArtifacts::default(),
    };

    let mut visitor = RustSymbolVisitor::new(file_path);
    visitor.visit_file(&parsed);
    let edges = visitor.local_call_edges();
    let symbols = visitor.symbols;
    RustArtifacts { symbols, edges, unresolved_calls: Vec::new() }
}

#[derive(Debug, Default)]
pub(crate) struct RustArtifacts {
    pub(crate) symbols: Vec<RustSymbol>,
    pub(crate) edges: Vec<RustGraphEdge>,
    /// Calls that couldn't be resolved within this file: (caller_id, callee_name, line)
    pub(crate) unresolved_calls: Vec<(String, String, Option<i64>)>,
}

fn build_rust_symbol(
    file_path: &str,
    kind: &'static str,
    name: &str,
    qualified_name: &str,
    parent_id: Option<String>,
    line_num: i64,
    signature: Option<String>,
    is_pub: bool,
    is_async: bool,
) -> RustSymbol {
    RustSymbol {
        id: generate_node_id(file_path, kind, name, line_num),
        kind,
        name: name.to_string(),
        qualified_name: qualified_name.to_string(),
        parent_id,
        file_path: file_path.to_string(),
        language: "rust",
        start_line: line_num,
        end_line: line_num,
        signature,
        visibility: Some(if is_pub { "public" } else { "private" }),
        is_exported: is_pub,
        is_async,
    }
}

pub(crate) fn generate_node_id(file_path: &str, kind: &str, name: &str, line: i64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{file_path}:{kind}:{name}:{line}").as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("{kind}:{}", &digest[..32])
}

fn unix_time_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

struct RustSymbolVisitor<'a> {
    file_path: &'a str,
    symbols: Vec<RustSymbol>,
    module_stack: Vec<ContainerFrame>,
    impl_stack: Vec<String>,
    type_ids: std::collections::HashMap<String, String>,
    scoped_symbol_ids: std::collections::HashMap<String, String>,
    local_calls: Vec<PendingCallEdge>,
    current_callable: Option<String>,
}

impl<'a> RustSymbolVisitor<'a> {
    fn new(file_path: &'a str) -> Self {
        Self {
            file_path,
            symbols: Vec::new(),
            module_stack: Vec::new(),
            impl_stack: Vec::new(),
            type_ids: std::collections::HashMap::new(),
            scoped_symbol_ids: std::collections::HashMap::new(),
            local_calls: Vec::new(),
            current_callable: None,
        }
    }

    fn push_item_symbol(
        &mut self,
        kind: &'static str,
        name: &str,
        line_num: i64,
        signature: Option<String>,
        visibility: Option<&syn::Visibility>,
        is_async: bool,
    ) -> String {
        let is_pub = matches!(visibility, Some(syn::Visibility::Public(_)));
        let parent_id = if kind == "method" {
            self.impl_stack
                .last()
                .and_then(|receiver| self.type_ids.get(receiver).cloned())
        } else {
            self.module_stack.last().map(|frame| frame.id.clone())
        };
        let qualified_name = if kind == "method" {
            let receiver = self
                .impl_stack
                .last()
                .cloned()
                .unwrap_or_else(|| "Unknown".to_string());
            format!("{receiver}::{name}")
        } else {
            let module_prefix = if self.module_stack.is_empty() {
                String::new()
            } else {
                let names = self
                    .module_stack
                    .iter()
                    .map(|frame| frame.name.as_str())
                    .collect::<Vec<_>>()
                    .join("::");
                format!("{names}::")
            };
            format!("{}::{}{}", self.file_path, module_prefix, name)
        };

        let symbol = build_rust_symbol(
            self.file_path,
            kind,
            name,
            &qualified_name,
            parent_id,
            line_num,
            signature,
            is_pub,
            is_async,
        );
        let id = symbol.id.clone();
        if matches!(kind, "struct" | "enum" | "trait" | "type_alias") {
            self.type_ids.insert(name.to_string(), id.clone());
        }
        if matches!(kind, "function" | "method" | "struct" | "enum" | "trait" | "type_alias" | "module") {
            self.scoped_symbol_ids.insert(qualified_name.clone(), id.clone());
        }
        self.symbols.push(symbol);
        id
    }

    fn current_scope_prefix(&self) -> Option<String> {
        if let Some(receiver) = self.impl_stack.last() {
            return Some(receiver.clone());
        }
        if self.module_stack.is_empty() {
            return None;
        }
        Some(self.module_stack.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join("::"))
    }

    fn register_call(&mut self, callee_name: String, line: i64) {
        if let Some(source_id) = self.current_callable.clone() {
            self.local_calls.push(PendingCallEdge {
                source_id,
                callee_name,
                scope_prefix: self.current_scope_prefix(),
                line: Some(line),
            });
        }
    }

    fn local_call_edges(&self) -> Vec<RustGraphEdge> {
        let mut edges = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for call in &self.local_calls {
            let candidates = [
                call.scope_prefix
                    .as_ref()
                    .map(|prefix| format!("{}::{}::{}", self.file_path, prefix, call.callee_name)),
                call.scope_prefix
                    .as_ref()
                    .map(|prefix| format!("{prefix}::{}", call.callee_name)),
                Some(format!("{}::{}", self.file_path, call.callee_name)),
            ];

            let mut target_id = None;
            for candidate in candidates.into_iter().flatten() {
                if let Some(id) = self.scoped_symbol_ids.get(&candidate) {
                    target_id = Some(id.clone());
                    break;
                }
            }
            if let Some(target) = target_id {
                let key = (call.source_id.clone(), target.clone(), call.line.unwrap_or_default());
                if seen.insert(key) {
                    edges.push(RustGraphEdge {
                        source: call.source_id.clone(),
                        target,
                        kind: "calls",
                        line: call.line,
                    });
                }
            }
        }
        edges
    }
}

#[derive(Debug, Clone)]
struct ContainerFrame {
    id: String,
    name: String,
}

#[derive(Debug, Clone)]
struct PendingCallEdge {
    source_id: String,
    callee_name: String,
    scope_prefix: Option<String>,
    line: Option<i64>,
}

impl<'ast> Visit<'ast> for RustSymbolVisitor<'_> {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        let line_num = node.ident.span().start().line as i64;
        let module_id = self.push_item_symbol(
            "module",
            &node.ident.to_string(),
            line_num,
            None,
            Some(&node.vis),
            false,
        );

        if let Some((_, items)) = &node.content {
            self.module_stack.push(ContainerFrame {
                id: module_id,
                name: node.ident.to_string(),
            });
            for item in items {
                self.visit_item(item);
            }
            self.module_stack.pop();
        }
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        let line_num = node.sig.ident.span().start().line as i64;
        let id = self.push_item_symbol(
            "function",
            &node.sig.ident.to_string(),
            line_num,
            Some(node.sig.to_token_stream().to_string()),
            Some(&node.vis),
            node.sig.asyncness.is_some(),
        );
        let previous = self.current_callable.replace(id);
        syn::visit::visit_block(self, &node.block);
        self.current_callable = previous;
    }

    fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
        let line_num = node.ident.span().start().line as i64;
        self.push_item_symbol(
            "struct",
            &node.ident.to_string(),
            line_num,
            None,
            Some(&node.vis),
            false,
        );
    }

    fn visit_item_enum(&mut self, node: &'ast syn::ItemEnum) {
        let line_num = node.ident.span().start().line as i64;
        self.push_item_symbol(
            "enum",
            &node.ident.to_string(),
            line_num,
            None,
            Some(&node.vis),
            false,
        );
    }

    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        let line_num = node.ident.span().start().line as i64;
        self.push_item_symbol(
            "trait",
            &node.ident.to_string(),
            line_num,
            None,
            Some(&node.vis),
            false,
        );
    }

    fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
        let line_num = node.ident.span().start().line as i64;
        self.push_item_symbol(
            "type_alias",
            &node.ident.to_string(),
            line_num,
            None,
            Some(&node.vis),
            false,
        );
    }

    fn visit_item_const(&mut self, node: &'ast syn::ItemConst) {
        let line_num = node.ident.span().start().line as i64;
        self.push_item_symbol(
            "constant",
            &node.ident.to_string(),
            line_num,
            None,
            Some(&node.vis),
            false,
        );
    }

    fn visit_item_static(&mut self, node: &'ast syn::ItemStatic) {
        let line_num = node.ident.span().start().line as i64;
        self.push_item_symbol(
            "variable",
            &node.ident.to_string(),
            line_num,
            None,
            Some(&node.vis),
            false,
        );
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        let receiver = node.self_ty.to_token_stream().to_string().replace(' ', "");
        self.impl_stack.push(receiver);
        syn::visit::visit_item_impl(self, node);
        self.impl_stack.pop();
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        let line_num = node.sig.ident.span().start().line as i64;
        let id = self.push_item_symbol(
            "method",
            &node.sig.ident.to_string(),
            line_num,
            Some(node.sig.to_token_stream().to_string()),
            Some(&node.vis),
            node.sig.asyncness.is_some(),
        );
        let previous = self.current_callable.replace(id);
        syn::visit::visit_block(self, &node.block);
        self.current_callable = previous;
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path_expr) = &*node.func {
            if let Some(segment) = path_expr.path.segments.last() {
                self.register_call(
                    segment.ident.to_string(),
                    node.span().start().line as i64,
                );
            }
        }
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        self.register_call(node.method.to_string(), node.span().start().line as i64);
        syn::visit::visit_expr_method_call(self, node);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use rusqlite::Connection;

    use super::{detect_language, index_project, looks_like_cpp, sync_project};
    use crate::config::create_default_config;
    use crate::db::{get_database_path, initialize_database};

    #[test]
    fn detects_basic_languages() {
        assert_eq!(detect_language("src/main.rs", None), "rust");
        assert_eq!(detect_language("src/app.tsx", None), "tsx");
        assert_eq!(detect_language("templates/theme.liquid", None), "liquid");
    }

    #[test]
    fn upgrades_header_to_cpp_when_source_looks_like_cpp() {
        let header = "namespace demo { class Foo {}; }";
        assert!(looks_like_cpp(header));
        assert_eq!(detect_language("include/foo.h", Some(header)), "cpp");
    }

    #[test]
    fn indexes_rust_files_into_sqlite() {
        let project_root = temp_project_root("indexes_rust_files_into_sqlite");
        fs::create_dir_all(project_root.join("src")).unwrap();
        fs::write(
            project_root.join("src/lib.rs"),
            r#"
pub struct User;
pub enum Role { Admin, User }
pub trait Service { fn run(&self); }
pub type UserId = u64;
pub const DEFAULT_NAME: &str = "guest";
pub static GLOBAL_COUNT: usize = 0;

pub fn top_level(a: i32) {}

impl User {
    pub async fn save(&self) {}
    fn private_helper(&self) {}
}
"#,
        )
        .unwrap();

        initialize_database(&project_root).unwrap();
        let config = create_default_config(&project_root);
        let summary = index_project(&project_root, &config).unwrap();

        assert_eq!(summary.files_indexed, 1);
        assert_eq!(summary.rust_files_indexed, 1);
        assert!(summary.nodes_created >= 8);
        assert!(summary.edges_created >= 7);

        let conn = open_db(&project_root);
        let node_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))
            .unwrap();
        let edge_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))
            .unwrap();
        let file_row: (String, i64) = conn
            .query_row(
                "SELECT language, node_count FROM files WHERE path = 'src/lib.rs'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert_eq!(file_row.0, "rust");
        assert_eq!(file_row.1, node_count);
        assert_eq!(edge_count, node_count - 1);

        let names = load_node_names(&conn);
        assert!(names.contains(&("User".to_string(), "struct".to_string())));
        assert!(names.contains(&("top_level".to_string(), "function".to_string())));
        assert!(names.contains(&("save".to_string(), "method".to_string())));
        assert!(names.contains(&("private_helper".to_string(), "method".to_string())));
        assert!(names.contains(&("DEFAULT_NAME".to_string(), "constant".to_string())));
    }

    #[test]
    fn indexes_python_files_with_symbols() {
        let project_root = temp_project_root("indexes_python_files_with_symbols");
        fs::create_dir_all(project_root.join("src")).unwrap();
        fs::write(
            project_root.join("src/app.py"),
            "def hello():\n    return 'hi'\n",
        )
        .unwrap();

        initialize_database(&project_root).unwrap();
        let config = create_default_config(&project_root);
        let summary = index_project(&project_root, &config).unwrap();

        assert_eq!(summary.files_indexed, 1);
        assert_eq!(summary.rust_files_indexed, 0);
        // file node + at least the hello() function
        assert!(summary.nodes_created >= 2, "expected file + function nodes, got {}", summary.nodes_created);

        let conn = open_db(&project_root);
        let kinds: Vec<String> = conn
            .prepare("SELECT kind FROM nodes ORDER BY kind")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();

        assert!(kinds.contains(&"file".to_string()));
        assert!(kinds.contains(&"function".to_string()));
    }

    #[test]
    fn indexes_nested_rust_modules_with_qualified_names() {
        let project_root = temp_project_root("indexes_nested_rust_modules_with_qualified_names");
        fs::create_dir_all(project_root.join("src")).unwrap();
        fs::write(
            project_root.join("src/lib.rs"),
            r#"
pub mod api {
    pub struct Client;

    impl Client {
        pub fn send(&self) {}
    }
}
"#,
        )
        .unwrap();

        initialize_database(&project_root).unwrap();
        let config = create_default_config(&project_root);
        index_project(&project_root, &config).unwrap();

        let conn = open_db(&project_root);
        let rows: Vec<(String, String, String)> = conn
            .prepare("SELECT kind, name, qualified_name FROM nodes ORDER BY kind, name")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();

        assert!(rows.contains(&(
            "module".to_string(),
            "api".to_string(),
            "src/lib.rs::api".to_string()
        )));
        assert!(rows.contains(&(
            "struct".to_string(),
            "Client".to_string(),
            "src/lib.rs::api::Client".to_string()
        )));
        assert!(rows.contains(&(
            "method".to_string(),
            "send".to_string(),
            "Client::send".to_string()
        )));

        let edges: Vec<(String, String)> = conn
            .prepare("SELECT source, target FROM edges WHERE kind = 'contains' ORDER BY source, target")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();

        let module_id: String = conn
            .query_row(
                "SELECT id FROM nodes WHERE kind = 'module' AND name = 'api'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let struct_id: String = conn
            .query_row(
                "SELECT id FROM nodes WHERE kind = 'struct' AND name = 'Client'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let method_id: String = conn
            .query_row(
                "SELECT id FROM nodes WHERE kind = 'method' AND name = 'send'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert!(edges.contains(&(module_id.clone(), struct_id.clone())));
        assert!(edges.contains(&(struct_id, method_id)));
    }

    #[test]
    fn indexes_local_call_edges_for_rust_functions() {
        let project_root = temp_project_root("indexes_local_call_edges_for_rust_functions");
        fs::create_dir_all(project_root.join("src")).unwrap();
        fs::write(
            project_root.join("src/lib.rs"),
            r#"
pub fn callee() {}

pub fn caller() {
    callee();
}

pub struct Client;
impl Client {
    pub fn send(&self) {}
    pub fn run(&self) {
        self.send();
    }
}
"#,
        )
        .unwrap();

        initialize_database(&project_root).unwrap();
        let config = create_default_config(&project_root);
        index_project(&project_root, &config).unwrap();

        let conn = open_db(&project_root);
        let edges: Vec<(String, String, String)> = conn
            .prepare("SELECT source, target, kind FROM edges WHERE kind = 'calls' ORDER BY source, target")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();

        let caller_id: String = conn
            .query_row("SELECT id FROM nodes WHERE kind = 'function' AND name = 'caller'", [], |row| row.get(0))
            .unwrap();
        let callee_id: String = conn
            .query_row("SELECT id FROM nodes WHERE kind = 'function' AND name = 'callee'", [], |row| row.get(0))
            .unwrap();
        let run_id: String = conn
            .query_row("SELECT id FROM nodes WHERE kind = 'method' AND name = 'run'", [], |row| row.get(0))
            .unwrap();
        let send_id: String = conn
            .query_row("SELECT id FROM nodes WHERE kind = 'method' AND name = 'send'", [], |row| row.get(0))
            .unwrap();

        assert!(edges.contains(&(caller_id, callee_id, "calls".to_string())));
        assert!(edges.contains(&(run_id, send_id, "calls".to_string())));
    }

    #[test]
    fn sync_project_reindexes_current_workspace() {
        let project_root = temp_project_root("sync_project_reindexes_current_workspace");
        fs::create_dir_all(project_root.join("src")).unwrap();
        fs::write(project_root.join("src/lib.rs"), "pub fn first() {}\n").unwrap();

        initialize_database(&project_root).unwrap();
        let config = create_default_config(&project_root);
        index_project(&project_root, &config).unwrap();

        fs::write(project_root.join("src/lib.rs"), "pub fn second() {}\n").unwrap();
        let summary = sync_project(&project_root, &config).unwrap();

        assert_eq!(summary.files_checked, 1);
        assert_eq!(summary.files_reindexed, 1);

        let conn = open_db(&project_root);
        let names = load_node_names(&conn);
        assert!(names.contains(&("second".to_string(), "function".to_string())));
        assert!(!names.contains(&("first".to_string(), "function".to_string())));
    }

    #[test]
    fn indexes_typescript_symbols_and_call_edges() {
        let project_root = temp_project_root("indexes_typescript_symbols_and_call_edges");
        fs::create_dir_all(project_root.join("src")).unwrap();
        fs::write(
            project_root.join("src/app.ts"),
            r#"
export interface ApiClient {}
export type UserId = string;
export enum Mode { A, B }

export function callee() {}
export function caller() {
  callee();
}

export const arrowFn = () => {
  callee();
};

export class Service {
  public send() {}
  public run() {
    send();
  }
}
"#,
        )
        .unwrap();

        initialize_database(&project_root).unwrap();
        let config = create_default_config(&project_root);
        let summary = index_project(&project_root, &config).unwrap();

        assert_eq!(summary.files_indexed, 1);
        assert_eq!(summary.rust_files_indexed, 0);
        assert!(summary.nodes_created >= 8);

        let conn = open_db(&project_root);
        let names = load_node_names(&conn);
        assert!(names.contains(&("ApiClient".to_string(), "interface".to_string())));
        assert!(names.contains(&("UserId".to_string(), "type_alias".to_string())));
        assert!(names.contains(&("Mode".to_string(), "enum".to_string())));
        assert!(names.contains(&("callee".to_string(), "function".to_string())));
        assert!(names.contains(&("caller".to_string(), "function".to_string())));
        assert!(names.contains(&("arrowFn".to_string(), "function".to_string())));
        assert!(names.contains(&("Service".to_string(), "class".to_string())));
        assert!(names.contains(&("send".to_string(), "method".to_string())));
        assert!(names.contains(&("run".to_string(), "method".to_string())));

        let edges: Vec<(String, String, String)> = conn
            .prepare("SELECT source, target, kind FROM edges WHERE kind = 'calls' ORDER BY source, target")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();

        let caller_id: String = conn
            .query_row("SELECT id FROM nodes WHERE kind = 'function' AND name = 'caller'", [], |row| row.get(0))
            .unwrap();
        let callee_id: String = conn
            .query_row("SELECT id FROM nodes WHERE kind = 'function' AND name = 'callee'", [], |row| row.get(0))
            .unwrap();
        let run_id: String = conn
            .query_row("SELECT id FROM nodes WHERE kind = 'method' AND name = 'run'", [], |row| row.get(0))
            .unwrap();
        let send_id: String = conn
            .query_row("SELECT id FROM nodes WHERE kind = 'method' AND name = 'send'", [], |row| row.get(0))
            .unwrap();

        assert!(edges.contains(&(caller_id.clone(), callee_id.clone(), "calls".to_string())));
        assert!(edges.iter().any(|(src, dst, kind)| src == &run_id && dst == &send_id && kind == "calls"));
    }

    fn load_node_names(conn: &Connection) -> Vec<(String, String)> {
        conn.prepare("SELECT name, kind FROM nodes ORDER BY name")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    fn open_db(project_root: &Path) -> Connection {
        Connection::open(get_database_path(project_root)).unwrap()
    }

    fn temp_project_root(test_name: &str) -> PathBuf {
        let unique = format!(
            "codegraph-rs-{}-{}",
            test_name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(root.join(".codegraph")).unwrap();
        root
    }

    // ── tree-sitter multi-language extraction tests ───────────────────────────

    fn extract_kinds_and_names(root: &Path) -> Vec<(String, String)> {
        let conn = Connection::open(get_database_path(root)).unwrap();
        let mut stmt = conn
            .prepare("SELECT kind, name FROM nodes WHERE kind != 'file' ORDER BY kind, name")
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    fn index_single_file(test_name: &str, filename: &str, content: &str) -> PathBuf {
        let root = temp_project_root(test_name);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src").join(filename), content).unwrap();
        initialize_database(&root).unwrap();
        let config = create_default_config(&root);
        index_project(&root, &config).unwrap();
        root
    }

    #[test]
    fn extracts_python_classes_and_methods() {
        let content = r#"
class Dog:
    def bark(self):
        pass

    def fetch(self):
        pass

def standalone():
    pass
"#;
        let root = index_single_file("py_classes_methods", "pet.py", content);
        let nodes = extract_kinds_and_names(&root);
        let kinds: Vec<&str> = nodes.iter().map(|(k, _)| k.as_str()).collect();
        let names: Vec<&str> = nodes.iter().map(|(_, n)| n.as_str()).collect();
        assert!(kinds.contains(&"class"), "expected class node");
        assert!(kinds.contains(&"method"), "expected method node");
        assert!(kinds.contains(&"function"), "expected function node");
        assert!(names.contains(&"Dog"), "expected Dog class");
        assert!(names.contains(&"bark"), "expected bark method");
        assert!(names.contains(&"standalone"), "expected standalone function");
    }

    #[test]
    fn extracts_go_functions_and_structs() {
        let content = r#"
package main

type Server struct {
    addr string
}

func (s *Server) Start() {}

func main() {}
"#;
        let root = index_single_file("go_funcs_structs", "main.go", content);
        let nodes = extract_kinds_and_names(&root);
        let kinds: Vec<&str> = nodes.iter().map(|(k, _)| k.as_str()).collect();
        let names: Vec<&str> = nodes.iter().map(|(_, n)| n.as_str()).collect();
        assert!(kinds.contains(&"struct"), "expected struct node");
        assert!(kinds.contains(&"method"), "expected method node");
        assert!(kinds.contains(&"function"), "expected function node");
        assert!(names.contains(&"Server"), "expected Server struct");
        assert!(names.contains(&"main"), "expected main function");
    }

    #[test]
    fn extracts_typescript_classes_and_functions() {
        let content = r#"
export class UserService {
    async getUser(id: string): Promise<User> {
        return fetch(id);
    }
}

export function formatName(first: string, last: string): string {
    return `${first} ${last}`;
}
"#;
        let root = index_single_file("ts_class_fn", "service.ts", content);
        let nodes = extract_kinds_and_names(&root);
        let kinds: Vec<&str> = nodes.iter().map(|(k, _)| k.as_str()).collect();
        let names: Vec<&str> = nodes.iter().map(|(_, n)| n.as_str()).collect();
        assert!(kinds.contains(&"class"), "expected class");
        assert!(kinds.contains(&"method"), "expected method");
        assert!(kinds.contains(&"function"), "expected function");
        assert!(names.contains(&"UserService"), "expected UserService class");
        assert!(names.contains(&"formatName"), "expected formatName function");
    }

    #[test]
    fn extracts_java_class_and_methods() {
        let content = r#"
public class Animal {
    public void eat() {}
    private void sleep() {}
}
"#;
        let root = index_single_file("java_class_methods", "Animal.java", content);
        let nodes = extract_kinds_and_names(&root);
        let kinds: Vec<&str> = nodes.iter().map(|(k, _)| k.as_str()).collect();
        let names: Vec<&str> = nodes.iter().map(|(_, n)| n.as_str()).collect();
        assert!(kinds.contains(&"class"), "expected class");
        assert!(kinds.contains(&"method"), "expected method");
        assert!(names.contains(&"Animal"), "expected Animal class");
        assert!(names.contains(&"eat"), "expected eat method");
    }

    #[test]
    fn resolves_imports_edges_for_typescript() {
        let root = temp_project_root("ts_import_resolution");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/utils.ts"),
            "export function formatDate(d: Date): string { return d.toISOString(); }\n",
        )
        .unwrap();
        fs::write(
            root.join("src/app.ts"),
            "import { formatDate } from './utils';\nexport function run() { formatDate(new Date()); }\n",
        )
        .unwrap();

        initialize_database(&root).unwrap();
        let config = create_default_config(&root);
        let summary = index_project(&root, &config).unwrap();

        // Should have created at least one imports edge
        let conn = open_db(&root);
        let import_edges: Vec<(String, String)> = {
            let mut stmt = conn
                .prepare("SELECT source, target FROM edges WHERE kind = 'imports'")
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert!(
            !import_edges.is_empty(),
            "expected imports edge from app.ts to utils.ts, summary={:?}", summary
        );
    }

    #[test]
    fn resolves_imports_edges_for_python() {
        let root = temp_project_root("py_import_resolution");
        fs::create_dir_all(root.join("mypackage")).unwrap();
        fs::write(root.join("mypackage/utils.py"), "def helper(): pass\n").unwrap();
        fs::write(
            root.join("mypackage/app.py"),
            "from mypackage.utils import helper\n\ndef main(): helper()\n",
        )
        .unwrap();

        initialize_database(&root).unwrap();
        let config = create_default_config(&root);
        index_project(&root, &config).unwrap();

        let conn = open_db(&root);
        let import_edges: Vec<(String, String)> = {
            let mut stmt = conn
                .prepare("SELECT source, target FROM edges WHERE kind = 'imports'")
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert!(
            !import_edges.is_empty(),
            "expected imports edge from app.py to utils.py"
        );
    }

    #[test]
    fn resolves_cross_file_calls_via_imports() {
        let root = temp_project_root("cross_file_calls");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/math.ts"),
            "export function add(a: number, b: number): number { return a + b; }\n",
        )
        .unwrap();
        fs::write(
            root.join("src/app.ts"),
            "import { add } from './math';\nexport function run() { return add(1, 2); }\n",
        )
        .unwrap();

        initialize_database(&root).unwrap();
        let config = create_default_config(&root);
        index_project(&root, &config).unwrap();

        let conn = open_db(&root);

        // Verify the cross-file calls edge: run() → add()
        let calls_edges: Vec<(String, String)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT n1.name, n2.name
                       FROM edges e
                       JOIN nodes n1 ON n1.id = e.source
                       JOIN nodes n2 ON n2.id = e.target
                      WHERE e.kind = 'calls'",
                )
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };

        assert!(
            calls_edges
                .iter()
                .any(|(caller, callee)| caller == "run" && callee == "add"),
            "expected cross-file calls edge run→add, got: {:?}",
            calls_edges
        );
    }

    #[test]
    fn extracts_express_route_nodes() {
        let content = r#"
const express = require('express');
const app = express();

app.get('/users', getUsers);
app.post('/users', createUser);
app.delete('/users/:id', deleteUser);
"#;
        let root = index_single_file("express_routes", "app.js", content);
        let conn = open_db(&root);
        let route_names: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM nodes WHERE kind = 'route' ORDER BY name")
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert!(
            route_names.contains(&"/users".to_string()),
            "expected /users route, got: {:?}",
            route_names
        );
        assert!(
            route_names.len() >= 2,
            "expected at least 2 routes, got: {:?}",
            route_names
        );
    }

    #[test]
    fn extracts_fastapi_route_nodes() {
        let content = r#"
from fastapi import FastAPI
app = FastAPI()

@app.get("/items")
async def list_items():
    return []

@app.post("/items")
async def create_item():
    return {}
"#;
        let root = index_single_file("fastapi_routes", "main.py", content);
        let conn = open_db(&root);
        let route_names: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM nodes WHERE kind = 'route' ORDER BY name")
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert!(
            route_names.contains(&"/items".to_string()),
            "expected /items route, got: {:?}",
            route_names
        );
        assert_eq!(route_names.len(), 2, "expected 2 routes, got: {:?}", route_names);
    }

    #[test]
    fn resolves_tsconfig_path_aliases() {
        let root = temp_project_root("tsconfig_aliases");
        fs::create_dir_all(root.join("src/utils")).unwrap();

        // Write tsconfig with @/ alias
        fs::write(
            root.join("tsconfig.json"),
            r#"{ "compilerOptions": { "baseUrl": ".", "paths": { "@/*": ["src/*"] } } }"#,
        )
        .unwrap();

        fs::write(
            root.join("src/utils/format.ts"),
            "export function format(s: string): string { return s.trim(); }\n",
        )
        .unwrap();
        fs::write(
            root.join("src/app.ts"),
            "import { format } from '@/utils/format';\nexport function run() { format('hi'); }\n",
        )
        .unwrap();

        initialize_database(&root).unwrap();
        let config = create_default_config(&root);
        index_project(&root, &config).unwrap();

        let conn = open_db(&root);
        let import_edges: Vec<(String, String)> = {
            let mut stmt = conn
                .prepare("SELECT source, target FROM edges WHERE kind = 'imports'")
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert!(
            !import_edges.is_empty(),
            "expected imports edge resolved through @/ alias"
        );
    }

    #[test]
    fn extracts_svelte_component() {
        let content = r#"<script lang="ts">
  export function greet(name: string): string { return `hi ${name}`; }
</script>
<template><h1>Hello</h1></template>"#;
        let root = index_single_file("svelte_component", "Greeting.svelte", content);
        let nodes = extract_kinds_and_names(&root);
        let kinds: Vec<&str> = nodes.iter().map(|(k, _)| k.as_str()).collect();
        let names: Vec<&str> = nodes.iter().map(|(_, n)| n.as_str()).collect();
        assert!(kinds.contains(&"component"), "expected component node");
        assert!(kinds.contains(&"function"), "expected function inside script");
        assert!(names.contains(&"Greeting"), "expected component named after file");
    }

    #[test]
    fn extracts_vue_component() {
        let content = r#"<script>
export function setup() { return {}; }
</script>
<template><div /></template>"#;
        let root = index_single_file("vue_component", "MyWidget.vue", content);
        let nodes = extract_kinds_and_names(&root);
        let kinds: Vec<&str> = nodes.iter().map(|(k, _)| k.as_str()).collect();
        let names: Vec<&str> = nodes.iter().map(|(_, n)| n.as_str()).collect();
        assert!(kinds.contains(&"component"), "expected component node");
        assert!(names.contains(&"MyWidget"), "expected component named after file");
    }
}
