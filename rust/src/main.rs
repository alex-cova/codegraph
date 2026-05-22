use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{ArgAction, Parser, Subcommand};
use codegraph_rs::{
    config, db, directory, extraction, graph::GraphService, install, mcp::McpServer,
    query::QueryService,
};

#[derive(Parser)]
#[command(name = "codegraph-rs")]
#[command(about = "Rust migration slice for CodeGraph")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Init {
        path: Option<PathBuf>,
        #[arg(short, long, action = ArgAction::SetTrue)]
        index: bool,
    },
    Uninit {
        path: Option<PathBuf>,
        #[arg(short, long, action = ArgAction::SetTrue)]
        force: bool,
    },
    Status {
        path: Option<PathBuf>,
        #[arg(long, action = ArgAction::SetTrue)]
        json: bool,
    },
    Index {
        path: Option<PathBuf>,
        #[arg(long, action = ArgAction::SetTrue)]
        json: bool,
    },
    Sync {
        path: Option<PathBuf>,
    },
    Query {
        search: String,
        path: Option<PathBuf>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        language: Option<String>,
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    Context {
        node_id: String,
        path: Option<PathBuf>,
        #[arg(long, action = ArgAction::SetTrue)]
        json: bool,
    },
    Scan {
        path: Option<PathBuf>,
        #[arg(long, action = ArgAction::SetTrue)]
        json: bool,
    },
    Files {
        path: Option<PathBuf>,
        #[arg(long)]
        filter: Option<String>,
        #[arg(short, long, action = ArgAction::SetTrue)]
        json: bool,
    },
    Affected {
        files: Vec<String>,
        path: Option<PathBuf>,
        #[arg(long, action = ArgAction::SetTrue)]
        stdin: bool,
        #[arg(short, long, default_value_t = 5)]
        depth: usize,
        #[arg(short = 'f', long)]
        filter: Option<String>,
        #[arg(short, long, action = ArgAction::SetTrue)]
        json: bool,
        #[arg(short, long, action = ArgAction::SetTrue)]
        quiet: bool,
    },
    Install {
        #[arg(short, long)]
        target: Option<String>,
        #[arg(short, long)]
        location: Option<String>,
        #[arg(short, long, action = ArgAction::SetTrue)]
        yes: bool,
        #[arg(long = "no-permissions", action = ArgAction::SetTrue)]
        no_permissions: bool,
        #[arg(long = "print-config")]
        print_config: Option<String>,
        #[arg(long, action = ArgAction::SetTrue)]
        uninstall: bool,
    },
    Unlock {
        path: Option<PathBuf>,
    },
    Serve {
        path: Option<PathBuf>,
        #[arg(long, action = ArgAction::SetTrue)]
        mcp: bool,
        #[arg(long = "no-watch", action = ArgAction::SetTrue)]
        no_watch: bool,
    },
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Init { path, index }) => init_command(path.as_deref(), index),
        Some(Commands::Uninit { path, force }) => uninit_command(path.as_deref(), force),
        Some(Commands::Status { path, json }) => status_command(path.as_deref(), json),
        Some(Commands::Index { path, json }) => index_command(path.as_deref(), json),
        Some(Commands::Sync { path }) => sync_command(path.as_deref()),
        Some(Commands::Files { path, filter, json }) => {
            files_command(path.as_deref(), filter.as_deref(), json)
        }
        Some(Commands::Affected {
            files,
            path,
            stdin,
            depth,
            filter,
            json,
            quiet,
        }) => affected_command(
            &files,
            path.as_deref(),
            stdin,
            depth,
            filter.as_deref(),
            json,
            quiet,
        ),
        Some(Commands::Install {
            target,
            location,
            yes,
            no_permissions,
            print_config,
            uninstall,
        }) => install_command(
            target.as_deref(),
            location.as_deref(),
            yes,
            no_permissions,
            print_config.as_deref(),
            uninstall,
        ),
        Some(Commands::Unlock { path }) => unlock_command(path.as_deref()),
        Some(Commands::Serve {
            path,
            mcp,
            no_watch,
        }) => serve_command(path.as_deref(), mcp, no_watch),
        Some(Commands::Query {
            search,
            path,
            kind,
            language,
            limit,
        }) => query_command(&search, path.as_deref(), kind.as_deref(), language.as_deref(), limit),
        Some(Commands::Context { node_id, path, json }) => {
            context_command(&node_id, path.as_deref(), json)
        }
        Some(Commands::Scan { path, json }) => scan_command(path.as_deref(), json),
        None => {
            eprintln!("No command supplied.");
            eprintln!("Available Rust-migrated commands: init, uninit, status, scan, index, sync, query, files, affected, context, install, unlock, serve");
            Ok(())
        }
    }
}

fn init_command(path: Option<&Path>, index: bool) -> Result<()> {
    let project_root = path
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir().context("failed to resolve current directory")?);
    let project_root = std::fs::canonicalize(&project_root).unwrap_or(project_root);

    if directory::is_initialized(&project_root) {
        bail!("CodeGraph already initialized in {}", project_root.display());
    }

    directory::create_directory(&project_root)?;
    let config_value = config::create_default_config(&project_root);
    config::save_config(&project_root, &config_value)?;
    let info = db::initialize_database(&project_root)?;

    println!("Initialized CodeGraph in {}", project_root.display());
    println!("Config: {}", config::get_config_path(&project_root).display());
    println!("Database: {}", info.path.display());

    if index {
        let summary = extraction::index_project(&project_root, &config_value)?;
        println!("Indexed project in Rust mode");
        println!("Files indexed: {}", summary.files_indexed);
        println!("Rust files indexed: {}", summary.rust_files_indexed);
        println!("Nodes created: {}", summary.nodes_created);
        println!("Edges created: {}", summary.edges_created);
        println!("Current limitation: Rust uses AST extraction, TypeScript-family languages use heuristic extraction, and other languages are file-level only.");
    }

    Ok(())
}

fn uninit_command(path: Option<&Path>, force: bool) -> Result<()> {
    let project_root = resolve_project_path(path)?;
    if !directory::is_initialized(&project_root) {
        bail!("CodeGraph is not initialized in {}", project_root.display());
    }

    if !force && !confirm_delete(&project_root)? {
        println!("Cancelled");
        return Ok(());
    }

    directory::remove_directory(&project_root)?;
    println!("Removed CodeGraph from {}", project_root.display());
    Ok(())
}

fn status_command(path: Option<&Path>, json: bool) -> Result<()> {
    let project_root = resolve_project_path(path)?;
    let initialized = directory::is_initialized(&project_root);

    if !initialized {
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "initialized": false,
                    "projectPath": project_root,
                })
            );
        } else {
            println!("CodeGraph Status");
            println!();
            println!("Project: {}", project_root.display());
            println!("Initialized: no");
        }
        return Ok(());
    }

    let config_value = config::load_config(&project_root)?;
    let directory_errors = directory::validate_directory(&project_root)?;
    let service = QueryService::open(&project_root)?;
    let db_info = service.database_info();
    let stats = service.get_stats()?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "initialized": true,
                "projectPath": project_root,
                "configPath": config::get_config_path(&project_root),
                "databasePath": db_info.path,
                "dbSizeBytes": db_info.size_bytes,
                "schemaVersion": db_info.schema_version,
                "nodeCount": stats.node_count,
                "edgeCount": stats.edge_count,
                "fileCount": stats.file_count,
                "nodesByKind": stats.nodes_by_kind,
                "edgesByKind": stats.edges_by_kind,
                "filesByLanguage": stats.files_by_language,
                "languages": config_value.languages,
                "frameworks": config_value.frameworks,
                "directoryErrors": directory_errors,
            }))?
        );
        return Ok(());
    }

    println!("CodeGraph Status");
    println!();
    println!("Project: {}", project_root.display());
    println!("Config: {}", config::get_config_path(&project_root).display());
    println!("Database: {}", db_info.path.display());
    println!("DB Size: {:.2} MB", db_info.size_bytes as f64 / 1024.0 / 1024.0);
    println!("Schema Version: {}", db_info.schema_version);
    println!("Files: {}", stats.file_count);
    println!("Nodes: {}", stats.node_count);
    println!("Edges: {}", stats.edge_count);
    println!("Configured Languages: {}", config_value.languages.len());
    println!("Framework Hints: {}", config_value.frameworks.len());
    if !stats.nodes_by_kind.is_empty() {
        println!();
        println!("Nodes by Kind:");
        for (kind, count) in &stats.nodes_by_kind {
            println!("  {kind:15} {count}");
        }
    }
    if !stats.files_by_language.is_empty() {
        println!();
        println!("Files by Language:");
        for (language, count) in &stats.files_by_language {
            println!("  {language:15} {count}");
        }
    }
    println!(
        "Directory Validation: {}",
        if directory_errors.is_empty() { "ok" } else { "issues found" }
    );
    for error in directory_errors {
        println!("  - {error}");
    }
    println!();
    println!("Partially ported in Rust: indexing currently uses Rust AST extraction plus heuristic TypeScript-family extraction.");
    println!("Unported in Rust: tree-sitter extraction parity, watch mode, and installer/MCP feature parity");

    Ok(())
}

fn query_command(
    search: &str,
    path: Option<&Path>,
    kind: Option<&str>,
    language: Option<&str>,
    limit: usize,
) -> Result<()> {
    let project_root = resolve_project_path(path)?;
    if !directory::is_initialized(&project_root) {
        bail!("CodeGraph is not initialized in {}", project_root.display());
    }

    let service = QueryService::open(&project_root)?;
    let results = service.search_nodes(search, kind, language, limit)?;

    if results.is_empty() {
        println!("No matches.");
        return Ok(());
    }

    for node in results {
        println!(
            "{} [{}] {}:{}-{}",
            node.qualified_name, 
            serde_json::to_string(&node.kind)?.trim_matches('"'),
            node.file_path,
            node.start_line,
            node.end_line
        );
    }

    Ok(())
}

fn index_command(path: Option<&Path>, json: bool) -> Result<()> {
    let project_root = path
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir().context("failed to resolve current directory")?);
    let project_root = std::fs::canonicalize(&project_root).unwrap_or(project_root);

    if !directory::is_initialized(&project_root) {
        bail!("CodeGraph is not initialized in {}", project_root.display());
    }

    let config_value = config::load_config(&project_root)?;
    let summary = extraction::index_project(&project_root, &config_value)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
        return Ok(());
    }

    println!("Indexed project in Rust mode");
    println!();
    println!("Project: {}", project_root.display());
    println!("Files indexed: {}", summary.files_indexed);
    println!("Rust files indexed: {}", summary.rust_files_indexed);
    println!("Nodes created: {}", summary.nodes_created);
    println!("Edges created: {}", summary.edges_created);
    println!();
    println!("Current limitation: Rust uses AST extraction, TypeScript-family languages use heuristic extraction, and other languages are file-level only.");

    Ok(())
}

fn sync_command(path: Option<&Path>) -> Result<()> {
    let project_root = resolve_project_path(path)?;
    if !directory::is_initialized(&project_root) {
        bail!("CodeGraph is not initialized in {}", project_root.display());
    }

    let config_value = config::load_config(&project_root)?;
    let summary = extraction::sync_project(&project_root, &config_value)?;

    println!("Synced project in Rust mode");
    println!();
    println!("Project: {}", project_root.display());
    println!("Files checked: {}", summary.files_checked);
    println!("Files reindexed: {}", summary.files_reindexed);
    println!("Rust files indexed: {}", summary.rust_files_indexed);
    println!("Nodes updated: {}", summary.nodes_updated);
    println!("Edges updated: {}", summary.edges_updated);
    println!("Current limitation: Rust sync is a full refresh, not incremental yet.");

    Ok(())
}

fn serve_command(path: Option<&Path>, mcp: bool, no_watch: bool) -> Result<()> {
    if no_watch {
        std::env::set_var("CODEGRAPH_NO_WATCH", "1");
    }

    let project_root = path.map(Path::to_path_buf);

    if mcp {
        let mut server = McpServer::new(project_root);
        return server.start();
    }

    eprintln!("CodeGraph MCP Server");
    eprintln!();
    eprintln!("Use --mcp flag to start the MCP server");
    eprintln!();
    eprintln!("Available tools:");
    eprintln!("  codegraph_search");
    eprintln!("  codegraph_context");
    eprintln!("  codegraph_callers");
    eprintln!("  codegraph_callees");
    eprintln!("  codegraph_impact");
    eprintln!("  codegraph_explore");
    eprintln!("  codegraph_node");
    eprintln!("  codegraph_files");
    eprintln!("  codegraph_status");
    Ok(())
}

fn install_command(
    target: Option<&str>,
    location: Option<&str>,
    yes: bool,
    no_permissions: bool,
    print_config: Option<&str>,
    uninstall: bool,
) -> Result<()> {
    let (target, location, auto_allow, init_local_project) =
        resolve_install_inputs(target, location, yes, no_permissions, print_config)?;
    let location = install::parse_location(Some(location.as_str()), yes)?;
    let print_target = print_config.map(install::parse_target_id).transpose()?;
    let targets = if let Some(print_target) = print_target {
        vec![print_target]
    } else {
        install::parse_targets(Some(target.as_str()))?
    };

    let install_options = install::InstallOptions {
        targets,
        location,
        auto_allow,
        print_config: print_target,
        uninstall,
    };
    install::run_install(install_options)?;

    if !uninstall && init_local_project && location == install::InstallLocation::Local {
        let project_root = std::env::current_dir().context("failed to resolve current directory")?;
        if directory::is_initialized(&project_root) {
            println!("CodeGraph already initialized in this project.");
        } else {
            init_command(Some(&project_root), true)?;
        }
    }

    Ok(())
}

fn unlock_command(path: Option<&Path>) -> Result<()> {
    let project_root = resolve_project_path(path)?;
    if !directory::is_initialized(&project_root) {
        bail!("CodeGraph is not initialized in {}", project_root.display());
    }
    let lock_path = directory::get_codegraph_dir(&project_root).join("codegraph.lock");
    if !lock_path.exists() {
        println!("No lock file found.");
        return Ok(());
    }
    std::fs::remove_file(&lock_path)
        .with_context(|| format!("failed to remove {}", lock_path.display()))?;
    println!("Removed lock file. You can now run indexing again.");
    Ok(())
}

fn resolve_install_inputs(
    target: Option<&str>,
    location: Option<&str>,
    yes: bool,
    no_permissions: bool,
    print_config: Option<&str>,
) -> Result<(String, String, bool, bool)> {
    if yes || print_config.is_some() || target.is_some() || location.is_some() {
        return Ok((
            target.unwrap_or("auto").to_string(),
            location.unwrap_or("global").to_string(),
            !no_permissions,
            false,
        ));
    }

    println!("CodeGraph installer");
    println!();
    println!("Targets: claude, cursor, codex, opencode");
    let target = prompt_with_default("Target agents (comma-separated, or auto/all/none)", "auto")?;
    let targets = install::parse_targets(Some(&target))?;
    let all_global_only = targets.iter().all(|target| matches!(target, install::InstallTarget::Codex));
    let default_location = if all_global_only { "global" } else { "global" };
    let location = prompt_with_default("Install location (global/local)", default_location)?;
    let auto_allow = if targets.iter().any(|target| matches!(target, install::InstallTarget::Claude)) {
        prompt_yes_no("Auto-allow CodeGraph commands for Claude?", true)?
    } else {
        false
    };
    let init_local_project = if location == "local" {
        prompt_yes_no("Initialize and index this project now?", true)?
    } else {
        false
    };
    Ok((target, location, auto_allow, init_local_project))
}

fn prompt_with_default(message: &str, default: &str) -> Result<String> {
    print!("{message} [{default}]: ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    let trimmed = answer.trim();
    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

fn prompt_yes_no(message: &str, default: bool) -> Result<bool> {
    let suffix = if default { "Y/n" } else { "y/N" };
    print!("{message} [{suffix}]: ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    let trimmed = answer.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Ok(default);
    }
    Ok(matches!(trimmed.as_str(), "y" | "yes"))
}

fn context_command(node_id: &str, path: Option<&Path>, json: bool) -> Result<()> {
    let project_root = resolve_project_path(path)?;
    if !directory::is_initialized(&project_root) {
        bail!("CodeGraph is not initialized in {}", project_root.display());
    }

    let service = QueryService::open(&project_root)?;
    let graph = GraphService::new(&service);
    let context = graph.get_context(node_id)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&context)?);
        return Ok(());
    }

    println!("Focal: {} [{}]", context.focal.qualified_name, serde_json::to_string(&context.focal.kind)?.trim_matches('"'));
    println!("File: {}", context.focal.file_path);
    println!();

    println!("Ancestors: {}", context.ancestors.len());
    for node in &context.ancestors {
        println!("  {} [{}]", node.qualified_name, serde_json::to_string(&node.kind)?.trim_matches('"'));
    }
    println!();

    println!("Children: {}", context.children.len());
    for node in &context.children {
        println!("  {} [{}]", node.qualified_name, serde_json::to_string(&node.kind)?.trim_matches('"'));
    }
    println!();

    println!("Incoming Refs: {}", context.incoming_refs.len());
    println!("Outgoing Refs: {}", context.outgoing_refs.len());
    println!("Types: {}", context.types.len());
    println!("Imports: {}", context.imports.len());

    Ok(())
}

fn scan_command(path: Option<&Path>, json: bool) -> Result<()> {
    let project_root = path
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir().context("failed to resolve current directory")?);
    let project_root = std::fs::canonicalize(&project_root).unwrap_or(project_root);

    let config_value = config::load_config(&project_root)?;
    let summary = extraction::scan_summary(&project_root, &config_value)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
        return Ok(());
    }

    println!("CodeGraph Scan");
    println!();
    println!("Project: {}", project_root.display());
    println!("Files: {}", summary.file_count);
    println!();
    println!("Files by Language:");
    for (language, count) in summary.files_by_language {
        println!("  {language:15} {count}");
    }

    Ok(())
}

fn files_command(path: Option<&Path>, filter: Option<&str>, json: bool) -> Result<()> {
    let project_root = resolve_project_path(path)?;
    if !directory::is_initialized(&project_root) {
        bail!("CodeGraph is not initialized in {}", project_root.display());
    }

    let service = QueryService::open(&project_root)?;
    let mut files = service.get_all_files()?;
    if let Some(filter) = filter {
        files.retain(|file| file.path.starts_with(filter));
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&files)?);
        return Ok(());
    }

    if files.is_empty() {
        println!("No indexed files found.");
        return Ok(());
    }

    println!("Indexed Files ({})", files.len());
    println!();
    for file in files {
        println!(
            "{} [{}] {} symbols",
            file.path,
            serde_json::to_string(&file.language)?.trim_matches('"'),
            file.node_count
        );
    }
    Ok(())
}

fn affected_command(
    file_args: &[String],
    path: Option<&Path>,
    use_stdin: bool,
    depth: usize,
    filter: Option<&str>,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let project_root = resolve_project_path(path)?;
    if !directory::is_initialized(&project_root) {
        bail!("CodeGraph is not initialized in {}", project_root.display());
    }

    let mut changed_files = file_args.to_vec();
    if use_stdin {
        let mut input = String::new();
        io::stdin().read_to_string(&mut input)?;
        changed_files.extend(
            input.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(ToOwned::to_owned),
        );
    }

    if changed_files.is_empty() {
        if !quiet {
            println!("No files provided.");
        }
        return Ok(());
    }

    let service = QueryService::open(&project_root)?;
    let graph = GraphService::new(&service);
    let mut affected_tests = std::collections::BTreeSet::new();
    let mut traversed = std::collections::BTreeSet::new();

    for changed in &changed_files {
        if is_test_file(changed, filter) {
            affected_tests.insert(changed.clone());
            continue;
        }

        let mut queue = std::collections::VecDeque::from([(changed.clone(), 0usize)]);
        let mut visited = std::collections::BTreeSet::from([changed.clone()]);
        while let Some((current, current_depth)) = queue.pop_front() {
            if current_depth >= depth {
                continue;
            }
            for dependent in graph.get_file_dependents(&current)? {
                if !visited.insert(dependent.clone()) {
                    continue;
                }
                traversed.insert(dependent.clone());
                if is_test_file(&dependent, filter) {
                    affected_tests.insert(dependent);
                } else {
                    queue.push_back((dependent, current_depth + 1));
                }
            }
        }
    }

    let tests = affected_tests.into_iter().collect::<Vec<_>>();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "changedFiles": changed_files,
                "affectedTests": tests,
                "totalDependentsTraversed": traversed.len(),
            }))?
        );
        return Ok(());
    }

    if quiet {
        for test in tests {
            println!("{test}");
        }
        return Ok(());
    }

    if tests.is_empty() {
        println!("No test files affected by the changed files.");
        return Ok(());
    }

    println!("Affected test files ({})", tests.len());
    println!();
    for test in tests {
        println!("  {test}");
    }
    Ok(())
}

fn is_test_file(file_path: &str, custom_filter: Option<&str>) -> bool {
    if let Some(filter) = custom_filter {
        return file_path.contains(filter.trim_matches('*'));
    }
    let patterns = [
        ".spec.",
        ".test.",
        "/__tests__/",
        "/tests/",
        "/test/",
        "/e2e/",
        "/spec/",
    ];
    patterns.iter().any(|pattern| file_path.contains(pattern))
}

fn resolve_project_path(path: Option<&Path>) -> Result<PathBuf> {
    let start = path
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir().context("failed to resolve current directory")?);
    let absolute = std::fs::canonicalize(&start).unwrap_or(start);
    if directory::is_initialized(&absolute) {
        return Ok(absolute);
    }

    let mut current = absolute.as_path();
    while let Some(parent) = current.parent() {
        if directory::is_initialized(parent) {
            return Ok(parent.to_path_buf());
        }
        current = parent;
    }

    Ok(absolute)
}

fn confirm_delete(project_root: &Path) -> Result<bool> {
    print!(
        "This will permanently delete all CodeGraph data in {}. Continue? (y/N) ",
        project_root.display()
    );
    io::stdout().flush()?;

    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(answer.trim().eq_ignore_ascii_case("y"))
}
