use std::path::Path;

use anyhow::Result;
use regex::Regex;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

// ── public entry point ────────────────────────────────────────────────────────

/// Scan all indexed source files for framework-specific route declarations and
/// insert `route` nodes + `references` edges into the database.
/// Returns (nodes_created, edges_created).
pub fn extract_framework_routes(conn: &Connection, project_root: &Path) -> Result<(usize, usize)> {
    // Load all files we need to scan (language-filtered)
    struct FileRow {
        path: String,
        language: String,
    }
    let files: Vec<FileRow> = {
        let mut stmt = conn.prepare(
            "SELECT path, language FROM files
              WHERE language IN ('javascript','typescript','tsx','jsx','python','java','go','ruby','php','csharp')",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(FileRow {
                    path: row.get(0)?,
                    language: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    // Build name→id map for existing function/method nodes (for references edges)
    let name_to_id: std::collections::HashMap<String, String> = {
        let mut stmt = conn.prepare(
            "SELECT name, id FROM nodes WHERE kind IN ('function','method','class')",
        )?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().collect()
    };

    let now = unix_time_ms();
    let mut nodes_created = 0usize;
    let mut edges_created = 0usize;

    for file in &files {
        let full_path = project_root.join(&file.path);
        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let routes = extract_routes(&file.path, &file.language, &content);
        for route in routes {
            let inserted = conn.execute(
                "INSERT OR IGNORE INTO nodes (
                     id, kind, name, qualified_name, file_path, language,
                     start_line, end_line, start_column, end_column,
                     docstring, signature, visibility,
                     is_exported, is_async, is_static, is_abstract,
                     decorators, type_parameters, updated_at
                 ) VALUES (
                     ?1, 'route', ?2, ?3, ?4, ?5,
                     ?6, ?6, 0, 0,
                     NULL, ?7, NULL,
                     0, 0, 0, 0,
                     NULL, NULL, ?8
                 )",
                params![
                    route.id,
                    route.name,
                    route.qualified_name,
                    route.file_path,
                    route.language,
                    route.line,
                    route.signature,
                    now
                ],
            )?;
            nodes_created += inserted;

            if inserted > 0 {
                if let Some(handler) = &route.handler {
                    if let Some(target_id) = name_to_id.get(handler) {
                        let e = conn.execute(
                            "INSERT OR IGNORE INTO edges
                                 (source, target, kind, metadata, line, col, provenance)
                             VALUES (?1, ?2, 'references', NULL, ?3, NULL, 'framework')",
                            params![route.id, target_id, route.line],
                        )?;
                        edges_created += e;
                    }
                }
            }
        }
    }

    Ok((nodes_created, edges_created))
}

// ── internal route representation ─────────────────────────────────────────────

struct Route {
    id: String,
    name: String,
    qualified_name: String,
    file_path: String,
    language: &'static str,
    line: i64,
    signature: Option<String>, // "METHOD /path"
    handler: Option<String>,
}

fn make_route(
    file_path: &str,
    language: &'static str,
    url: &str,
    method: Option<&str>,
    handler: Option<&str>,
    line: i64,
) -> Route {
    let mut hasher = Sha256::new();
    hasher.update(format!("{}:route:{}:{}", file_path, url, line).as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    let id = format!("route:{}", &digest[..32]);

    let sig = method.map(|m| format!("{} {}", m.to_uppercase(), url));

    Route {
        id,
        name: url.to_string(),
        qualified_name: format!("{}::route::{}", file_path, url),
        file_path: file_path.to_string(),
        language,
        line,
        signature: sig,
        handler: handler.map(|h| {
            // strip trailing () and receiver (e.g. "controller.method" → "method")
            let cleaned = h.trim().trim_end_matches("()");
            cleaned.rsplit('.').next().unwrap_or(cleaned).to_string()
        }),
    }
}

// ── per-language route extractors ─────────────────────────────────────────────

fn extract_routes(file_path: &str, language: &str, content: &str) -> Vec<Route> {
    match language {
        "javascript" | "typescript" | "tsx" | "jsx" => extract_js_routes(file_path, content),
        "python" => extract_python_routes(file_path, content),
        "java" => extract_java_routes(file_path, content),
        "go" => extract_go_routes(file_path, content),
        "ruby" => extract_ruby_routes(file_path, content),
        "php" => extract_php_routes(file_path, content),
        "csharp" => extract_csharp_routes(file_path, content),
        _ => Vec::new(),
    }
}

// ── JavaScript/TypeScript: Express, Fastify, Koa, Hapi ───────────────────────

fn extract_js_routes(file_path: &str, content: &str) -> Vec<Route> {
    // Match: app.get('/path', handler), router.post('/path', handler), etc.
    // Also catches fastify.get, server.get, etc.
    let re = Regex::new(
        r#"(?m)[\w$]+\.(get|post|put|patch|delete|options|head|all|use)\s*\(\s*['"`]([^'"`]+)['"`]\s*,\s*([\w.]+)"#,
    )
    .unwrap();

    let mut routes = Vec::new();
    for cap in re.captures_iter(content) {
        let method = cap.get(1).map(|m| m.as_str()).unwrap_or("get");
        let url = cap.get(2).map(|m| m.as_str()).unwrap_or("/");
        let handler = cap.get(3).map(|m| m.as_str());
        let line = line_number(content, cap.get(0).unwrap().start());
        routes.push(make_route(file_path, "typescript", url, Some(method), handler, line));
    }

    // Also match: router.route('/path').get(handler) style (just capture the path)
    let re2 = Regex::new(r#"(?m)[\w$]+\.route\s*\(\s*['"`]([^'"`]+)['"`]\s*\)"#).unwrap();
    for cap in re2.captures_iter(content) {
        let url = cap.get(1).map(|m| m.as_str()).unwrap_or("/");
        let line = line_number(content, cap.get(0).unwrap().start());
        routes.push(make_route(file_path, "typescript", url, None, None, line));
    }

    routes
}

// ── Python: FastAPI, Flask, Django ───────────────────────────────────────────

fn extract_python_routes(file_path: &str, content: &str) -> Vec<Route> {
    let mut routes = Vec::new();

    // FastAPI/Flask decorators: @app.get('/path'), @router.post('/path'), @bp.route('/path')
    let re_decorator = Regex::new(
        r#"(?m)@[\w]+\.(get|post|put|patch|delete|options|head|route)\s*\(\s*['"]([^'"]+)['"]"#,
    )
    .unwrap();
    for cap in re_decorator.captures_iter(content) {
        let method = cap.get(1).map(|m| m.as_str()).unwrap_or("get");
        let url = cap.get(2).map(|m| m.as_str()).unwrap_or("/");
        let line = line_number(content, cap.get(0).unwrap().start());
        // Try to find the next function def after the decorator
        let handler = next_function_name(content, cap.get(0).unwrap().end());
        routes.push(make_route(
            file_path,
            "python",
            url,
            Some(method),
            handler.as_deref(),
            line,
        ));
    }

    // Django urls.py: path('url', handler), re_path(r'url', handler), url(r'url', handler)
    let re_django = Regex::new(
        r#"(?m)\b(?:path|re_path|url)\s*\(\s*r?['"]([^'"]+)['"]\s*,\s*([\w.]+)"#,
    )
    .unwrap();
    for cap in re_django.captures_iter(content) {
        let url = cap.get(1).map(|m| m.as_str()).unwrap_or("/");
        let handler = cap.get(2).map(|m| m.as_str());
        let line = line_number(content, cap.get(0).unwrap().start());
        routes.push(make_route(file_path, "python", url, None, handler, line));
    }

    routes
}

// ── Java: Spring MVC ─────────────────────────────────────────────────────────

fn extract_java_routes(file_path: &str, content: &str) -> Vec<Route> {
    let mut routes = Vec::new();

    // @GetMapping("/path"), @PostMapping("/path"), @RequestMapping("/path")
    let re = Regex::new(
        r#"(?m)@(Get|Post|Put|Patch|Delete|Request)Mapping\s*\(\s*(?:value\s*=\s*)?['"]([^'"]+)['"]"#,
    )
    .unwrap();
    for cap in re.captures_iter(content) {
        let verb = cap.get(1).map(|m| m.as_str()).unwrap_or("Request");
        let method = if verb == "Request" { None } else { Some(verb) };
        let url = cap.get(2).map(|m| m.as_str()).unwrap_or("/");
        let line = line_number(content, cap.get(0).unwrap().start());
        let handler = next_java_method(content, cap.get(0).unwrap().end());
        routes.push(make_route(
            file_path,
            "java",
            url,
            method,
            handler.as_deref(),
            line,
        ));
    }

    routes
}

// ── Go: Gin, net/http, Chi, Echo ─────────────────────────────────────────────

fn extract_go_routes(file_path: &str, content: &str) -> Vec<Route> {
    let mut routes = Vec::new();

    // Gin: r.GET("/path", handler), r.POST("/path", handler)
    // Chi/Echo/Fiber: similar
    let re_gin = Regex::new(
        r#"(?m)[\w]+\.(GET|POST|PUT|PATCH|DELETE|OPTIONS|HEAD|Any|Handle)\s*\(\s*"([^"]+)"\s*,\s*([\w.]+)"#,
    )
    .unwrap();
    for cap in re_gin.captures_iter(content) {
        let method = cap.get(1).map(|m| m.as_str()).unwrap_or("GET");
        let url = cap.get(2).map(|m| m.as_str()).unwrap_or("/");
        let handler = cap.get(3).map(|m| m.as_str());
        let line = line_number(content, cap.get(0).unwrap().start());
        routes.push(make_route(file_path, "go", url, Some(method), handler, line));
    }

    // net/http: mux.HandleFunc("/path", handler), http.HandleFunc("/path", handler)
    let re_stdlib = Regex::new(
        r#"(?m)[\w]+\.HandleFunc\s*\(\s*"([^"]+)"\s*,\s*([\w.]+)"#,
    )
    .unwrap();
    for cap in re_stdlib.captures_iter(content) {
        let url = cap.get(1).map(|m| m.as_str()).unwrap_or("/");
        let handler = cap.get(2).map(|m| m.as_str());
        let line = line_number(content, cap.get(0).unwrap().start());
        routes.push(make_route(file_path, "go", url, None, handler, line));
    }

    // Axum: .route("/path", get(handler))
    let re_axum = Regex::new(
        r#"(?m)\.route\s*\(\s*"([^"]+)"\s*,\s*(?:get|post|put|patch|delete)\s*\(\s*([\w:]+)"#,
    )
    .unwrap();
    for cap in re_axum.captures_iter(content) {
        let url = cap.get(1).map(|m| m.as_str()).unwrap_or("/");
        let handler = cap.get(2).map(|m| m.as_str());
        let line = line_number(content, cap.get(0).unwrap().start());
        routes.push(make_route(file_path, "go", url, None, handler, line));
    }

    routes
}

// ── Ruby: Rails ───────────────────────────────────────────────────────────────

fn extract_ruby_routes(file_path: &str, content: &str) -> Vec<Route> {
    let mut routes = Vec::new();

    // get '/path', to: 'controller#action'
    // post '/path', to: 'controller#action'
    let re_verb = Regex::new(
        r#"(?m)^\s*(get|post|put|patch|delete|options|resources?|namespace)\s+['"]([^'"]+)['"]"#,
    )
    .unwrap();
    for cap in re_verb.captures_iter(content) {
        let method = cap.get(1).map(|m| m.as_str()).unwrap_or("get");
        let url = cap.get(2).map(|m| m.as_str()).unwrap_or("/");
        let line = line_number(content, cap.get(0).unwrap().start());
        // Try to find controller#action
        let handler = extract_rails_handler(content, cap.get(0).unwrap().end());
        routes.push(make_route(
            file_path,
            "ruby",
            url,
            Some(method),
            handler.as_deref(),
            line,
        ));
    }

    routes
}

// ── PHP: Laravel ─────────────────────────────────────────────────────────────

fn extract_php_routes(file_path: &str, content: &str) -> Vec<Route> {
    let mut routes = Vec::new();

    // Route::get('/path', [Controller::class, 'method'])
    // Route::get('/path', 'handler')
    let re = Regex::new(
        r#"(?m)Route::(get|post|put|patch|delete|options|any)\s*\(\s*['"]([^'"]+)['"]\s*,"#,
    )
    .unwrap();
    for cap in re.captures_iter(content) {
        let method = cap.get(1).map(|m| m.as_str()).unwrap_or("get");
        let url = cap.get(2).map(|m| m.as_str()).unwrap_or("/");
        let line = line_number(content, cap.get(0).unwrap().start());
        routes.push(make_route(file_path, "php", url, Some(method), None, line));
    }

    routes
}

// ── C#: ASP.NET ───────────────────────────────────────────────────────────────

fn extract_csharp_routes(file_path: &str, content: &str) -> Vec<Route> {
    let mut routes = Vec::new();

    // [HttpGet("/path")], [Route("/path")]
    // [HttpPost("/path")]
    let re = Regex::new(
        r#"(?m)\[(Http(?:Get|Post|Put|Patch|Delete)|Route)\s*\(\s*['"]([^'"]+)['"]"#,
    )
    .unwrap();
    for cap in re.captures_iter(content) {
        let attr = cap.get(1).map(|m| m.as_str()).unwrap_or("Route");
        let method = attr.strip_prefix("Http");
        let url = cap.get(2).map(|m| m.as_str()).unwrap_or("/");
        let line = line_number(content, cap.get(0).unwrap().start());
        let handler = next_csharp_method(content, cap.get(0).unwrap().end());
        routes.push(make_route(
            file_path,
            "csharp",
            url,
            method,
            handler.as_deref(),
            line,
        ));
    }

    // Minimal API: app.MapGet("/path", handler)
    let re_minimal = Regex::new(
        r#"(?m)[\w]+\.Map(Get|Post|Put|Patch|Delete)\s*\(\s*['"]([^'"]+)['"]\s*,\s*([\w.]+)"#,
    )
    .unwrap();
    for cap in re_minimal.captures_iter(content) {
        let method = cap.get(1).map(|m| m.as_str()).unwrap_or("Get");
        let url = cap.get(2).map(|m| m.as_str()).unwrap_or("/");
        let handler = cap.get(3).map(|m| m.as_str());
        let line = line_number(content, cap.get(0).unwrap().start());
        routes.push(make_route(
            file_path,
            "csharp",
            url,
            Some(method),
            handler,
            line,
        ));
    }

    routes
}

// ── helper utilities ──────────────────────────────────────────────────────────

fn line_number(content: &str, byte_offset: usize) -> i64 {
    content[..byte_offset.min(content.len())]
        .chars()
        .filter(|&c| c == '\n')
        .count() as i64
        + 1
}

/// Find the name of the Python function defined on the line(s) immediately after `offset`.
fn next_function_name(content: &str, offset: usize) -> Option<String> {
    let rest = &content[offset.min(content.len())..];
    let re = Regex::new(r"^\s*\n\s*(?:async\s+)?def\s+(\w+)").unwrap();
    re.captures(rest)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Find the name of the Java method following an annotation.
fn next_java_method(content: &str, offset: usize) -> Option<String> {
    let rest = &content[offset.min(content.len())..];
    let re = Regex::new(r"(?s)[\w\s<>@,\[\]]*?\s+(\w+)\s*\(").unwrap();
    re.captures(rest)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Find the name of the C# method following an attribute.
fn next_csharp_method(content: &str, offset: usize) -> Option<String> {
    let rest = &content[offset.min(content.len())..];
    // Skip to end of )] then find next identifier before (
    let re = Regex::new(r"(?s)\]\s*(?:\[.*?\]\s*)*(?:public|private|protected|internal|static|async|virtual|override|\s)*\s+\w+\s+(\w+)\s*\(").unwrap();
    re.captures(rest)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Find the Rails `to:` value and extract the action name.
fn extract_rails_handler(content: &str, offset: usize) -> Option<String> {
    let rest = &content[offset.min(content.len())..];
    // to: 'controller#action'
    let re = Regex::new(r#"to:\s*['"][\w/]+#(\w+)['"]"#).unwrap();
    re.captures(rest)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

fn unix_time_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
