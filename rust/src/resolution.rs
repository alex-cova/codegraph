use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use rusqlite::{params, Connection};

// ── tsconfig path-alias types ─────────────────────────────────────────────────

/// A single pattern from `compilerOptions.paths`.
#[derive(Debug, Clone)]
pub struct AliasPattern {
    pub prefix: String,
    pub suffix: String,
    pub has_wildcard: bool,
    pub replacements: Vec<String>,
}

/// Parsed alias map from tsconfig / jsconfig.
#[derive(Debug, Clone)]
pub struct AliasMap {
    /// Directory that `compilerOptions.paths` is relative to (= baseUrl resolved).
    pub base_url: String,
    /// Patterns sorted most-specific-first.
    pub patterns: Vec<AliasPattern>,
}

/// Load `compilerOptions.paths` from `tsconfig.json` or `jsconfig.json` at
/// `project_root`. Returns `None` when no file is found or no `paths` block exists.
pub fn load_project_aliases(project_root: &Path) -> Option<AliasMap> {
    for name in &["tsconfig.json", "jsconfig.json"] {
        let p = project_root.join(name);
        if !p.exists() {
            continue;
        }
        let raw = std::fs::read_to_string(&p).ok()?;
        let stripped = strip_jsonc(&raw);
        let v: serde_json::Value = serde_json::from_str(&stripped).ok()?;

        let co = v.get("compilerOptions")?;
        let base_url_rel = co
            .get("baseUrl")
            .and_then(|b| b.as_str())
            .unwrap_or(".");
        let base_url = project_root
            .join(base_url_rel)
            .to_string_lossy()
            .replace('\\', "/");

        let paths = co.get("paths")?.as_object()?;
        if paths.is_empty() {
            continue;
        }

        let mut patterns: Vec<AliasPattern> = Vec::new();
        for (pattern, targets) in paths {
            let replacements: Vec<String> = targets
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| t.as_str().map(|s| s.to_string()))
                .collect();
            if replacements.is_empty() {
                continue;
            }
            let star = pattern.find('*');
            let (prefix, suffix, has_wildcard) = if let Some(idx) = star {
                (pattern[..idx].to_string(), pattern[idx + 1..].to_string(), true)
            } else {
                (pattern.clone(), String::new(), false)
            };
            patterns.push(AliasPattern { prefix, suffix, has_wildcard, replacements });
        }
        if patterns.is_empty() {
            continue;
        }
        // Sort: longer prefix first; literal before wildcard of same length
        patterns.sort_by(|a, b| {
            b.prefix.len()
                .cmp(&a.prefix.len())
                .then_with(|| a.has_wildcard.cmp(&b.has_wildcard))
        });

        return Some(AliasMap { base_url, patterns });
    }
    None
}

/// Apply an `AliasMap` to `import_path`. Returns candidate filesystem paths
/// (project-root-relative, forward slashes). Empty vec = no alias matched.
pub fn apply_aliases(import_path: &str, aliases: &AliasMap, project_root: &Path) -> Vec<String> {
    let root_str = project_root.to_string_lossy().replace('\\', "/");
    for pat in &aliases.patterns {
        if !import_path.starts_with(&pat.prefix) {
            continue;
        }
        if !pat.suffix.is_empty() && !import_path.ends_with(&pat.suffix) {
            continue;
        }
        let captured = if pat.has_wildcard {
            &import_path[pat.prefix.len()..import_path.len() - pat.suffix.len()]
        } else if import_path != pat.prefix {
            continue;
        } else {
            ""
        };
        let mut out = Vec::new();
        for target in &pat.replacements {
            let filled = if pat.has_wildcard {
                target.replace('*', captured)
            } else {
                target.clone()
            };
            // Resolve against base_url, then make relative to project root
            let absolute = if filled.starts_with('/') {
                filled.clone()
            } else {
                format!("{}/{}", aliases.base_url, filled)
            };
            let relative = if absolute.starts_with(&root_str) {
                absolute[root_str.len()..].trim_start_matches('/').to_string()
            } else {
                normalize_path(&absolute)
            };
            if !relative.starts_with("..") {
                out.push(relative.replace('\\', "/"));
            }
        }
        if !out.is_empty() {
            return out;
        }
    }
    Vec::new()
}

/// Strip `//` and `/* */` comments and trailing commas from JSONC so
/// `serde_json` can parse tsconfigs with VS Code annotations.
fn strip_jsonc(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let ch = chars[i];
        if in_string {
            out.push(ch);
            if ch == '\\' && i + 1 < chars.len() {
                i += 1;
                out.push(chars[i]);
            } else if ch == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if ch == '"' {
            in_string = true;
            out.push(ch);
            i += 1;
            continue;
        }
        if ch == '/' && i + 1 < chars.len() {
            if chars[i + 1] == '/' {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            if chars[i + 1] == '*' {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
                continue;
            }
        }
        out.push(ch);
        i += 1;
    }
    // Remove trailing commas before } or ]
    let re = regex::Regex::new(r",(\s*[}\]])").unwrap();
    re.replace_all(&out, "$1").into_owned()
}

// ── public entry point ────────────────────────────────────────────────────────

/// Resolve import nodes to their target file nodes and insert `imports` edges.
/// Returns the number of edges created.
pub fn resolve_imports(conn: &Connection, project_root: &Path) -> Result<usize> {
    // Load tsconfig/jsconfig path aliases once for the whole pass
    let alias_map = load_project_aliases(project_root);

    // Build a fast lookup set of all indexed file paths
    let known_files: HashSet<String> = {
        let mut stmt = conn.prepare("SELECT path FROM files")?;
        let rows = stmt.query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<HashSet<String>>>()?;
        rows
    };

    // Load all file node ids for path → id lookup
    let file_node_ids: std::collections::HashMap<String, String> = {
        let mut stmt =
            conn.prepare("SELECT file_path, id FROM nodes WHERE kind = 'file'")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<std::collections::HashMap<String, String>>>()?;
        rows
    };

    // Load all import nodes
    struct ImportNode {
        id: String,
        name: String,       // raw import path / module name
        file_path: String,  // project-relative path of the importing file
        language: String,
    }
    let import_nodes: Vec<ImportNode> = {
        let mut stmt = conn.prepare(
            "SELECT id, name, file_path, language FROM nodes WHERE kind = 'import'",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ImportNode {
                id: row.get(0)?,
                name: row.get(1)?,
                file_path: row.get(2)?,
                language: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    let mut edges_created = 0usize;

    for imp in &import_nodes {
        let resolved = resolve_import_path(
            &imp.name,
            &imp.file_path,
            &imp.language,
            &known_files,
            alias_map.as_ref(),
            project_root,
        );
        if let Some(target_path) = resolved {
            if let Some(target_id) = file_node_ids.get(&target_path) {
                let inserted = conn.execute(
                    "INSERT OR IGNORE INTO edges
                         (source, target, kind, metadata, line, col, provenance)
                     VALUES (?1, ?2, 'imports', NULL, NULL, NULL, 'resolution')",
                    params![imp.id, target_id],
                )?;
                edges_created += inserted;
            }
        }
    }

    Ok(edges_created)
}

// ── path resolution ───────────────────────────────────────────────────────────

fn resolve_import_path(
    import_path: &str,
    from_file: &str,
    language: &str,
    known_files: &HashSet<String>,
    aliases: Option<&AliasMap>,
    project_root: &Path,
) -> Option<String> {
    // Skip blank
    if import_path.is_empty() {
        return None;
    }

    let from_dir = Path::new(from_file)
        .parent()
        .map(|p| p.to_str().unwrap_or(""))
        .unwrap_or("");

    // Relative import (starts with ./ or ../)
    if import_path.starts_with('.') {
        return resolve_relative(import_path, from_dir, language, known_files);
    }

    // Try tsconfig/jsconfig path aliases first (before external check)
    if let Some(alias_map) = aliases {
        let candidates = apply_aliases(import_path, alias_map, project_root);
        for candidate in candidates {
            if let Some(resolved) = try_with_extensions(&candidate, language, known_files) {
                return Some(resolved);
            }
        }
    }

    // Skip external packages for each language
    if is_external(import_path, language) {
        return None;
    }

    // Python dotted module import (e.g. "mypackage.module")
    if language == "python" {
        return resolve_python_dotted(import_path, from_dir, known_files);
    }

    // Aliased import (@/, ~/, src/) — heuristic fallback when no tsconfig
    resolve_aliased(import_path, language, known_files)
}

fn resolve_relative(
    import_path: &str,
    from_dir: &str,
    language: &str,
    known_files: &HashSet<String>,
) -> Option<String> {
    let base = join_and_normalize(from_dir, import_path);
    try_with_extensions(&base, language, known_files)
}

fn resolve_python_dotted(
    import_path: &str,
    from_dir: &str,
    known_files: &HashSet<String>,
) -> Option<String> {
    // Convert "mypackage.module" → "mypackage/module"
    let slash_path = import_path.replace('.', "/");
    // Try relative to the importing file's directory first, then project root
    for prefix in &[from_dir, ""] {
        let base = if prefix.is_empty() {
            slash_path.clone()
        } else {
            format!("{}/{}", prefix, slash_path)
        };
        let normalized = normalize_path(&base);
        for ext in &[".py", "/__init__.py"] {
            let candidate = format!("{}{}", normalized, ext);
            if known_files.contains(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn resolve_aliased(
    import_path: &str,
    language: &str,
    known_files: &HashSet<String>,
) -> Option<String> {
    // Common project alias conventions
    const ALIASES: &[(&str, &str)] = &[
        ("@/", "src/"),
        ("~/", "src/"),
        ("@src/", "src/"),
        ("src/", "src/"),
        ("@app/", "app/"),
        ("app/", "app/"),
    ];
    for (prefix, replacement) in ALIASES {
        if import_path.starts_with(prefix) {
            let base = import_path.replacen(prefix, replacement, 1);
            if let Some(resolved) = try_with_extensions(&base, language, known_files) {
                return Some(resolved);
            }
        }
    }
    None
}

fn try_with_extensions(
    base: &str,
    language: &str,
    known_files: &HashSet<String>,
) -> Option<String> {
    // Try without adding extension first (import might already have one)
    if known_files.contains(base) {
        return Some(base.to_string());
    }
    for ext in extension_order(language) {
        let candidate = format!("{}{}", base, ext);
        if known_files.contains(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn extension_order(language: &str) -> &'static [&'static str] {
    match language {
        "typescript" => &[".ts", ".tsx", ".d.ts", ".js", ".jsx", "/index.ts", "/index.tsx", "/index.js"],
        "tsx" => &[".tsx", ".ts", ".d.ts", ".js", ".jsx", "/index.tsx", "/index.ts", "/index.js"],
        "javascript" => &[".js", ".jsx", ".mjs", ".cjs", "/index.js", "/index.jsx"],
        "jsx" => &[".jsx", ".js", "/index.jsx", "/index.js"],
        "python" => &[".py", "/__init__.py"],
        "go" => &[".go"],
        "java" => &[".java"],
        "csharp" => &[".cs"],
        "php" => &[".php"],
        "ruby" => &[".rb"],
        "c" => &[".c", ".h"],
        "cpp" => &[".cpp", ".cc", ".cxx", ".h", ".hpp"],
        _ => &[],
    }
}

// ── external package detection ────────────────────────────────────────────────

fn is_external(import_path: &str, language: &str) -> bool {
    // Relative imports are always local
    if import_path.starts_with('.') {
        return false;
    }
    // Common alias prefixes that signal local imports
    if import_path.starts_with("@/")
        || import_path.starts_with("~/")
        || import_path.starts_with("src/")
        || import_path.starts_with("app/")
    {
        return false;
    }

    match language {
        "typescript" | "tsx" | "javascript" | "jsx" => {
            // Node built-ins
            const NODE_BUILTINS: &[&str] = &[
                "fs", "path", "os", "crypto", "http", "https", "url", "util",
                "events", "stream", "child_process", "buffer", "net", "tls",
                "readline", "zlib", "querystring", "string_decoder", "assert",
                "v8", "vm", "worker_threads", "cluster", "dns", "dgram",
                "process", "module", "console",
            ];
            let root = import_path.split('/').next().unwrap_or(import_path);
            if NODE_BUILTINS.contains(&root) || import_path.starts_with("node:") {
                return true;
            }
            // Scoped npm packages (@org/pkg) or bare names (react, lodash)
            // that don't match a known local alias pattern
            if import_path.starts_with('@') && !import_path.starts_with("@/") {
                return true;
            }
            // Bare specifier with no slash prefix = npm package
            !import_path.contains('/')
        }
        "python" => {
            const PYTHON_STDLIB: &[&str] = &[
                "os", "sys", "json", "re", "math", "datetime", "collections",
                "typing", "pathlib", "logging", "io", "abc", "copy",
                "functools", "itertools", "operator", "string", "struct",
                "time", "random", "hashlib", "urllib", "http", "email",
                "xml", "csv", "configparser", "argparse", "unittest",
                "dataclasses", "enum", "contextlib", "threading", "multiprocessing",
                "subprocess", "socket", "ssl", "select", "queue", "heapq",
                "bisect", "weakref", "gc", "inspect", "ast", "dis",
                "traceback", "warnings", "tempfile", "shutil", "glob",
                "fnmatch", "stat", "pickle", "shelve", "sqlite3",
                "base64", "binascii", "codecs", "locale", "gettext",
                "pprint", "textwrap", "difflib",
            ];
            let top = import_path.split('.').next().unwrap_or(import_path);
            PYTHON_STDLIB.contains(&top)
        }
        "go" => {
            // Go: imports with no '/' are stdlib, imports with domain-style paths are external
            // Only relative (./...) are local in Go
            true // we already handled relative above; all non-relative Go imports are external
        }
        "java" => {
            // java.*, javax.*, org.*, com.* that aren't relative are external
            import_path.starts_with("java.")
                || import_path.starts_with("javax.")
                || import_path.starts_with("android.")
                || import_path.starts_with("kotlin.")
        }
        "csharp" => {
            // System.*, Microsoft.* are external
            import_path.starts_with("System")
                || import_path.starts_with("Microsoft")
                || import_path.starts_with("global::")
        }
        "ruby" => {
            // Single-word requires without path components are gems
            !import_path.contains('/')
        }
        "php" => false, // PHP includes are file paths, almost always local
        "c" | "cpp" => {
            // Angle-bracket includes are system; but the text stored already strips the <>
            // Heuristic: no directory component = system header
            !import_path.contains('/')
        }
        _ => false,
    }
}

// ── path utilities ────────────────────────────────────────────────────────────

/// Join a directory prefix with a (possibly relative) path, then normalize.
fn join_and_normalize(dir: &str, path: &str) -> String {
    let joined = if dir.is_empty() {
        path.to_string()
    } else {
        format!("{}/{}", dir, path)
    };
    normalize_path(&joined)
}

/// Collapse `./`, `../`, and double slashes in a relative path string without
/// touching the filesystem. The result is always a clean forward-slash path.
fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

// ── cross-file call resolution ────────────────────────────────────────────────

/// For each unresolved call `(caller_id, callee_name, line)`:
/// 1. Look up all project symbols with that name (function/method/class).
/// 2. For each candidate, check if the caller's file has an `imports` edge to
///    the candidate's file — i.e., the caller explicitly imported that file.
/// 3. If so, create a `calls` edge.
///
/// Returns the number of new edges created.
pub fn resolve_cross_file_calls(
    conn: &Connection,
    unresolved: &[(String, String, Option<i64>)],
) -> Result<usize> {
    if unresolved.is_empty() {
        return Ok(0);
    }

    // Build: caller_id → file_path
    let caller_file: std::collections::HashMap<String, String> = {
        let ids: Vec<String> = unresolved
            .iter()
            .map(|(id, _, _)| id.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let mut map = std::collections::HashMap::new();
        for id in &ids {
            let fp: rusqlite::Result<String> = conn.query_row(
                "SELECT file_path FROM nodes WHERE id = ?1",
                params![id],
                |row| row.get(0),
            );
            if let Ok(fp) = fp {
                map.insert(id.clone(), fp);
            }
        }
        map
    };

    // Build: file_path → set of imported file_paths (via imports edges)
    // We query lazily per unique caller file below; cache the results.
    let mut import_cache: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();

    let mut edges_created = 0usize;

    for (caller_id, callee_name, line) in unresolved {
        let caller_fp = match caller_file.get(caller_id) {
            Some(fp) => fp.clone(),
            None => continue,
        };

        // Populate import cache for this file if needed
        if !import_cache.contains_key(&caller_fp) {
            let imported = get_imported_files(conn, &caller_fp);
            import_cache.insert(caller_fp.clone(), imported);
        }
        let imported = &import_cache[&caller_fp];

        if imported.is_empty() {
            continue;
        }

        // Find all symbols with this name in the imported files
        for target_file in imported {
            // Query for a function/method/class in target_file with this name
            let target_id: rusqlite::Result<String> = conn.query_row(
                "SELECT id FROM nodes
                  WHERE name = ?1
                    AND file_path = ?2
                    AND kind IN ('function', 'method', 'class', 'struct')
                  LIMIT 1",
                params![callee_name, target_file],
                |row| row.get(0),
            );
            if let Ok(target_id) = target_id {
                let inserted = conn.execute(
                    "INSERT OR IGNORE INTO edges
                         (source, target, kind, metadata, line, col, provenance)
                     VALUES (?1, ?2, 'calls', NULL, ?3, NULL, 'resolution')",
                    params![caller_id, target_id, line],
                )?;
                edges_created += inserted;
                break; // one edge per (caller, callee_name) is enough
            }
        }
    }

    Ok(edges_created)
}

/// Get the set of file paths that `file_path` imports (via `imports` edges).
fn get_imported_files(conn: &Connection, file_path: &str) -> std::collections::HashSet<String> {
    // import node → imports edge → file node → file_path
    let mut stmt = match conn.prepare(
        "SELECT n2.file_path
           FROM nodes n1
           JOIN edges e ON e.source = n1.id AND e.kind = 'imports'
           JOIN nodes n2 ON n2.id = e.target AND n2.kind = 'file'
          WHERE n1.kind = 'import'
            AND n1.file_path = ?1",
    ) {
        Ok(s) => s,
        Err(_) => return std::collections::HashSet::new(),
    };
    let rows = match stmt.query_map(params![file_path], |row| row.get::<_, String>(0)) {
        Ok(r) => r,
        Err(_) => return std::collections::HashSet::new(),
    };
    rows.flatten().collect()
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_simple_relative() {
        assert_eq!(normalize_path("src/./utils/../lib/foo"), "src/lib/foo");
    }

    #[test]
    fn normalize_up_from_root() {
        assert_eq!(normalize_path("src/../foo"), "foo");
    }

    #[test]
    fn join_relative_import() {
        assert_eq!(join_and_normalize("src/components", "./Button"), "src/components/Button");
        assert_eq!(join_and_normalize("src/components", "../utils"), "src/utils");
    }

    #[test]
    fn external_npm_detection() {
        assert!(is_external("react", "typescript"));
        assert!(is_external("@org/package", "typescript"));
        assert!(!is_external("@/components/Button", "typescript"));
        assert!(!is_external("./Button", "typescript"));
    }

    #[test]
    fn external_python_stdlib() {
        assert!(is_external("os", "python"));
        assert!(is_external("os.path", "python"));
        assert!(!is_external("mypackage.utils", "python"));
    }

    #[test]
    fn external_go_is_all_non_relative() {
        assert!(is_external("fmt", "go"));
        assert!(is_external("github.com/foo/bar", "go"));
        assert!(!is_external("./internal/util", "go"));
    }

    #[test]
    fn resolve_python_dotted_import() {
        let mut known = HashSet::new();
        known.insert("mypackage/utils.py".to_string());
        let result = resolve_python_dotted("mypackage.utils", "", &known);
        assert_eq!(result, Some("mypackage/utils.py".to_string()));
    }

    #[test]
    fn resolve_relative_typescript() {
        let mut known = HashSet::new();
        known.insert("src/utils/format.ts".to_string());
        let result = resolve_relative("./format", "src/utils", "typescript", &known);
        assert_eq!(result, Some("src/utils/format.ts".to_string()));
    }
}
