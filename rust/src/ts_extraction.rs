use crate::extraction::{generate_node_id, RustArtifacts, RustGraphEdge, RustSymbol};
use tree_sitter::{Language, Node, Parser};

// ── language config ──────────────────────────────────────────────────────────

struct LangConfig {
    function_types: &'static [&'static str],
    class_types: &'static [&'static str],
    method_types: &'static [&'static str],
    interface_types: &'static [&'static str],
    struct_types: &'static [&'static str],
    enum_types: &'static [&'static str],
    type_alias_types: &'static [&'static str],
    import_types: &'static [&'static str],
    call_types: &'static [&'static str],
}

fn get_config(language: &str) -> Option<&'static LangConfig> {
    Some(match language {
        "typescript" | "tsx" => &TYPESCRIPT_CONFIG,
        "javascript" | "jsx" => &JAVASCRIPT_CONFIG,
        "python" => &PYTHON_CONFIG,
        "go" => &GO_CONFIG,
        "java" => &JAVA_CONFIG,
        "c" => &C_CONFIG,
        "cpp" => &CPP_CONFIG,
        "csharp" => &CSHARP_CONFIG,
        "ruby" => &RUBY_CONFIG,
        "php" => &PHP_CONFIG,
        "swift" => &SWIFT_CONFIG,
        "lua" | "luau" => &LUA_CONFIG,
        "dart" => &DART_CONFIG,
        "scala" => &SCALA_CONFIG,
        _ => return None,
    })
}

fn get_ts_language(language: &str) -> Option<Language> {
    Some(match language {
        "typescript" => Language::from(tree_sitter_typescript::LANGUAGE_TYPESCRIPT),
        "tsx" => Language::from(tree_sitter_typescript::LANGUAGE_TSX),
        "javascript" | "jsx" => Language::from(tree_sitter_javascript::LANGUAGE),
        "python" => Language::from(tree_sitter_python::LANGUAGE),
        "go" => Language::from(tree_sitter_go::LANGUAGE),
        "java" => Language::from(tree_sitter_java::LANGUAGE),
        "c" => Language::from(tree_sitter_c::LANGUAGE),
        "cpp" => Language::from(tree_sitter_cpp::LANGUAGE),
        "csharp" => Language::from(tree_sitter_c_sharp::LANGUAGE),
        "ruby" => Language::from(tree_sitter_ruby::LANGUAGE),
        "php" => Language::from(tree_sitter_php::LANGUAGE_PHP),
        "swift" => Language::from(tree_sitter_swift::LANGUAGE),
        "lua" | "luau" => Language::from(tree_sitter_lua::LANGUAGE),
        "dart" => Language::from(tree_sitter_dart::LANGUAGE),
        "scala" => Language::from(tree_sitter_scala::LANGUAGE),
        _ => return None,
    })
}

fn static_language(language: &str) -> &'static str {
    match language {
        "typescript" => "typescript",
        "tsx" => "tsx",
        "javascript" => "javascript",
        "jsx" => "jsx",
        "python" => "python",
        "go" => "go",
        "java" => "java",
        "c" => "c",
        "cpp" => "cpp",
        "csharp" => "csharp",
        "ruby" => "ruby",
        "php" => "php",
        "swift" => "swift",
        "lua" | "luau" => "lua",
        "dart" => "dart",
        "scala" => "scala",
        _ => "unknown",
    }
}

// ── per-language static configs ───────────────────────────────────────────────

static TYPESCRIPT_CONFIG: LangConfig = LangConfig {
    function_types: &[
        "function_declaration",
        "function_expression",
        "generator_function_declaration",
        "generator_function",
        "arrow_function",
        "function",
    ],
    class_types: &["class_declaration", "class"],
    method_types: &[
        "method_definition",
        "public_field_definition",
        "abstract_method_signature",
    ],
    interface_types: &["interface_declaration"],
    struct_types: &[],
    enum_types: &["enum_declaration"],
    type_alias_types: &["type_alias_declaration"],
    import_types: &["import_statement"],
    call_types: &["call_expression"],
};

static JAVASCRIPT_CONFIG: LangConfig = LangConfig {
    function_types: &[
        "function_declaration",
        "function_expression",
        "generator_function_declaration",
        "generator_function",
        "arrow_function",
        "function",
    ],
    class_types: &["class_declaration", "class"],
    method_types: &["method_definition", "public_field_definition"],
    interface_types: &[],
    struct_types: &[],
    enum_types: &[],
    type_alias_types: &[],
    import_types: &["import_statement"],
    call_types: &["call_expression"],
};

static PYTHON_CONFIG: LangConfig = LangConfig {
    function_types: &["function_definition"],
    class_types: &["class_definition"],
    method_types: &[], // handled contextually: function_definition inside class → method
    interface_types: &[],
    struct_types: &[],
    enum_types: &[],
    type_alias_types: &[],
    import_types: &["import_statement", "import_from_statement"],
    call_types: &["call"],
};

static GO_CONFIG: LangConfig = LangConfig {
    function_types: &["function_declaration"],
    class_types: &[],
    method_types: &[], // method_declaration handled by Go-specific visitor
    interface_types: &[], // handled via type_spec → interface_type
    struct_types: &[],    // handled via type_spec → struct_type
    enum_types: &[],
    type_alias_types: &["type_alias"], // type_spec handled by Go-specific visitor
    import_types: &["import_declaration"],
    call_types: &["call_expression"],
};

static JAVA_CONFIG: LangConfig = LangConfig {
    function_types: &[],
    class_types: &[
        "class_declaration",
        "record_declaration",
        "annotation_type_declaration",
    ],
    method_types: &["method_declaration", "constructor_declaration"],
    interface_types: &["interface_declaration"],
    struct_types: &[],
    enum_types: &["enum_declaration"],
    type_alias_types: &[],
    import_types: &["import_declaration"],
    call_types: &["method_invocation"],
};

static C_CONFIG: LangConfig = LangConfig {
    function_types: &["function_definition"],
    class_types: &[],
    method_types: &[],
    interface_types: &[],
    struct_types: &["struct_specifier"],
    enum_types: &["enum_specifier"],
    type_alias_types: &["type_definition"],
    import_types: &["preproc_include"],
    call_types: &["call_expression"],
};

static CPP_CONFIG: LangConfig = LangConfig {
    function_types: &["function_definition"],
    class_types: &["class_specifier"],
    method_types: &["function_definition"], // inside class → method (handled contextually)
    interface_types: &[],
    struct_types: &["struct_specifier"],
    enum_types: &["enum_specifier"],
    type_alias_types: &["type_definition", "alias_declaration"],
    import_types: &["preproc_include"],
    call_types: &["call_expression"],
};

static CSHARP_CONFIG: LangConfig = LangConfig {
    function_types: &[],
    class_types: &["class_declaration", "record_declaration"],
    method_types: &["method_declaration", "constructor_declaration"],
    interface_types: &["interface_declaration"],
    struct_types: &["struct_declaration"],
    enum_types: &["enum_declaration"],
    type_alias_types: &[],
    import_types: &["using_directive"],
    call_types: &["invocation_expression"],
};

static RUBY_CONFIG: LangConfig = LangConfig {
    function_types: &["method", "singleton_method"],
    class_types: &["class", "singleton_class"],
    method_types: &[], // Ruby methods are already "method" nodes at any depth
    interface_types: &[],
    struct_types: &["module"],
    enum_types: &[],
    type_alias_types: &[],
    import_types: &["call"], // require / require_relative
    call_types: &["call"],
};

static PHP_CONFIG: LangConfig = LangConfig {
    function_types: &["function_definition"],
    class_types: &["class_declaration"],
    method_types: &["method_declaration"],
    interface_types: &["interface_declaration"],
    struct_types: &[],
    enum_types: &["enum_declaration"],
    type_alias_types: &[],
    import_types: &["include_expression", "require_expression"],
    call_types: &["function_call_expression"],
};

static SWIFT_CONFIG: LangConfig = LangConfig {
    function_types: &["function_declaration"],
    class_types: &["class_declaration"],
    method_types: &["function_declaration"], // inside class → method (contextual)
    interface_types: &["protocol_declaration"],
    struct_types: &["struct_declaration"],
    enum_types: &["enum_declaration"],
    type_alias_types: &["typealias_declaration"],
    import_types: &["import_declaration"],
    call_types: &["call_expression"],
};


static LUA_CONFIG: LangConfig = LangConfig {
    function_types: &["function_declaration", "local_function"],
    class_types: &[],
    method_types: &["function_declaration"], // inside table → method (contextual)
    interface_types: &[],
    struct_types: &[],
    enum_types: &[],
    type_alias_types: &[],
    import_types: &["call_expression"], // require('module')
    call_types: &["call_expression"],
};

static DART_CONFIG: LangConfig = LangConfig {
    function_types: &["function_signature", "function_declaration"],
    class_types: &["class_definition"],
    method_types: &["method_signature", "function_signature"],
    interface_types: &[],
    struct_types: &[],
    enum_types: &["enum_declaration"],
    type_alias_types: &["type_alias"],
    import_types: &["import_or_export"],
    call_types: &["invocation_expression"],
};

static SCALA_CONFIG: LangConfig = LangConfig {
    function_types: &["function_definition", "val_definition", "var_definition"],
    class_types: &["class_definition", "object_definition", "case_class_definition"],
    method_types: &["function_definition"],
    interface_types: &["trait_definition"],
    struct_types: &[],
    enum_types: &["enum_definition"],
    type_alias_types: &["type_definition"],
    import_types: &["import_declaration"],
    call_types: &["call_expression"],
};

// ── extraction context ────────────────────────────────────────────────────────

struct ScopeEntry {
    id: String,
    name: String,
    kind: &'static str,
}

struct PendingCall {
    source_id: String,
    callee: String,
    line: Option<i64>,
}

struct ExtractCtx<'a> {
    file_path: &'a str,
    source: &'a [u8],
    language: &'static str,
    config: &'static LangConfig,
    // scope stack: innermost last
    scope: Vec<ScopeEntry>,
    symbols: Vec<RustSymbol>,
    edges: Vec<RustGraphEdge>,
    calls: Vec<PendingCall>,
    unresolved_calls: Vec<(String, String, Option<i64>)>,
    // name → id for within-file call resolution
    name_to_id: std::collections::HashMap<String, String>,
}

impl<'a> ExtractCtx<'a> {
    fn new(
        file_path: &'a str,
        source: &'a [u8],
        language: &'static str,
        config: &'static LangConfig,
    ) -> Self {
        Self {
            file_path,
            source,
            language,
            config,
            scope: Vec::new(),
            symbols: Vec::new(),
            edges: Vec::new(),
            calls: Vec::new(),
            unresolved_calls: Vec::new(),
            name_to_id: std::collections::HashMap::new(),
        }
    }

    fn current_parent_id(&self) -> Option<String> {
        self.scope.last().map(|e| e.id.clone())
    }

    fn current_scope_kind(&self) -> Option<&'static str> {
        self.scope.last().map(|e| e.kind)
    }

    fn push_scope(&mut self, id: String, name: String, kind: &'static str) {
        self.scope.push(ScopeEntry { id, name, kind });
    }

    fn pop_scope(&mut self) {
        self.scope.pop();
    }

    fn qualified_name(&self, name: &str) -> String {
        if self.scope.is_empty() {
            format!("{}::{}", self.file_path, name)
        } else {
            let parts: Vec<&str> = self.scope.iter().map(|e| e.name.as_str()).collect();
            format!("{}::{}", parts.join("::"), name)
        }
    }

    fn node_text(&self, node: Node) -> &str {
        node.utf8_text(self.source).unwrap_or("")
    }

    fn register(&mut self, sym: RustSymbol) {
        self.name_to_id
            .insert(sym.qualified_name.clone(), sym.id.clone());
        self.name_to_id.insert(sym.name.clone(), sym.id.clone());
        self.symbols.push(sym);
    }
}

// ── main visitor ──────────────────────────────────────────────────────────────

pub(crate) fn extract_with_tree_sitter(
    file_path: &str,
    language: &str,
    content: &str,
) -> Option<RustArtifacts> {
    let ts_language = get_ts_language(language)?;
    let config = get_config(language)?;
    let lang_str = static_language(language);

    let mut parser = Parser::new();
    if parser.set_language(&ts_language).is_err() {
        return None;
    }

    let source = content.as_bytes();
    let tree = parser.parse(source, None)?;
    let root = tree.root_node();

    let mut ctx = ExtractCtx::new(file_path, source, lang_str, config);
    visit_node(&mut ctx, root, false);
    resolve_calls(&mut ctx);

    Some(RustArtifacts {
        symbols: ctx.symbols,
        edges: ctx.edges,
        unresolved_calls: ctx.unresolved_calls,
    })
}

fn visit_node(ctx: &mut ExtractCtx, node: Node, in_class: bool) {
    let kind = node.kind();

    // ── Python: function_definition in class context → method ─────────────
    // Must precede generic function handler since Python reuses the same node type
    if ctx.language == "python" && in_class && kind == "function_definition" {
        if let Some(name) = get_named_child_text(ctx, node, "name") {
            let line = node.start_position().row as i64 + 1;
            let end_line = node.end_position().row as i64 + 1;
            let parent_id = ctx.current_parent_id();
            let qname = ctx.qualified_name(&name);
            let id = generate_node_id(ctx.file_path, "method", &name, line);
            let sym = RustSymbol {
                id: id.clone(),
                kind: "method",
                name,
                qualified_name: qname,
                parent_id,
                file_path: ctx.file_path.to_string(),
                language: ctx.language,
                start_line: line,
                end_line,
                signature: None,
                visibility: None,
                is_exported: false,
                is_async: detect_async(ctx, node),
            };
            ctx.register(sym);
            let mname = ctx.symbols.last().unwrap().name.clone();
            ctx.push_scope(id, mname, "method");
            for i in 0..node.child_count() {
                visit_node(ctx, node.child(i).unwrap(), false);
            }
            ctx.pop_scope();
            return;
        }
    }

    // ── C++: function_definition in class context → method ───────────────
    // Must precede generic function handler since C++ reuses function_definition
    if ctx.language == "cpp" && in_class && kind == "function_definition" {
        if let Some(name) = get_cpp_function_name(ctx, node) {
            let line = node.start_position().row as i64 + 1;
            let end_line = node.end_position().row as i64 + 1;
            let parent_id = ctx.current_parent_id();
            let qname = ctx.qualified_name(&name);
            let id = generate_node_id(ctx.file_path, "method", &name, line);
            let sym = RustSymbol {
                id: id.clone(),
                kind: "method",
                name,
                qualified_name: qname,
                parent_id,
                file_path: ctx.file_path.to_string(),
                language: ctx.language,
                start_line: line,
                end_line,
                signature: None,
                visibility: None,
                is_exported: false,
                is_async: false,
            };
            ctx.register(sym);
            let mname = ctx.symbols.last().unwrap().name.clone();
            ctx.push_scope(id, mname, "method");
            for i in 0..node.child_count() {
                visit_node(ctx, node.child(i).unwrap(), false);
            }
            ctx.pop_scope();
            return;
        }
    }

    // ── functions ────────────────────────────────────────────────────────────
    if ctx.config.function_types.contains(&kind) {
        if let Some(name) = extract_function_name(ctx, node) {
            let line = node.start_position().row as i64 + 1;
            let end_line = node.end_position().row as i64 + 1;
            let is_exported = detect_exported(ctx, node);
            let is_async = detect_async(ctx, node);
            let parent_id = ctx.current_parent_id();
            let qname = ctx.qualified_name(&name);
            let id = generate_node_id(ctx.file_path, "function", &name, line);
            let sym = RustSymbol {
                id: id.clone(),
                kind: "function",
                name,
                qualified_name: qname,
                parent_id,
                file_path: ctx.file_path.to_string(),
                language: ctx.language,
                start_line: line,
                end_line,
                signature: None,
                visibility: Some(if is_exported { "public" } else { "private" }),
                is_exported,
                is_async,
            };
            ctx.register(sym);
            ctx.push_scope(id, "function".to_string(), "function");
            for i in 0..node.child_count() {
                visit_node(ctx, node.child(i).unwrap(), false);
            }
            ctx.pop_scope();
            return;
        }
    }

    // ── classes ──────────────────────────────────────────────────────────────
    if ctx.config.class_types.contains(&kind) {
        if let Some(name) = get_named_child_text(ctx, node, "name") {
            let line = node.start_position().row as i64 + 1;
            let end_line = node.end_position().row as i64 + 1;
            let is_exported = detect_exported(ctx, node);
            let parent_id = ctx.current_parent_id();
            let qname = ctx.qualified_name(&name);
            let id = generate_node_id(ctx.file_path, "class", &name, line);
            let sym = RustSymbol {
                id: id.clone(),
                kind: "class",
                name,
                qualified_name: qname,
                parent_id,
                file_path: ctx.file_path.to_string(),
                language: ctx.language,
                start_line: line,
                end_line,
                signature: None,
                visibility: Some(if is_exported { "public" } else { "private" }),
                is_exported,
                is_async: false,
            };
            ctx.register(sym);
            ctx.push_scope(id, ctx.symbols.last().unwrap().name.clone(), "class");
            for i in 0..node.child_count() {
                visit_node(ctx, node.child(i).unwrap(), true);
            }
            ctx.pop_scope();
            return;
        }
    }

    // ── methods ──────────────────────────────────────────────────────────────
    if in_class && ctx.config.method_types.contains(&kind) {
        if let Some(name) = extract_method_name(ctx, node) {
            let line = node.start_position().row as i64 + 1;
            let end_line = node.end_position().row as i64 + 1;
            let is_exported = detect_exported(ctx, node);
            let is_async = detect_async(ctx, node);
            let parent_id = ctx.current_parent_id();
            let qname = ctx.qualified_name(&name);
            let id = generate_node_id(ctx.file_path, "method", &name, line);
            let sym = RustSymbol {
                id: id.clone(),
                kind: "method",
                name,
                qualified_name: qname,
                parent_id,
                file_path: ctx.file_path.to_string(),
                language: ctx.language,
                start_line: line,
                end_line,
                signature: None,
                visibility: detect_visibility(ctx, node),
                is_exported,
                is_async,
            };
            ctx.register(sym);
            ctx.push_scope(id, ctx.symbols.last().unwrap().name.clone(), "method");
            for i in 0..node.child_count() {
                visit_node(ctx, node.child(i).unwrap(), false);
            }
            ctx.pop_scope();
            return;
        }
    }

    // ── interfaces ───────────────────────────────────────────────────────────
    if ctx.config.interface_types.contains(&kind) {
        if let Some(name) = get_named_child_text(ctx, node, "name") {
            let line = node.start_position().row as i64 + 1;
            let end_line = node.end_position().row as i64 + 1;
            let is_exported = detect_exported(ctx, node);
            let parent_id = ctx.current_parent_id();
            let qname = ctx.qualified_name(&name);
            let id = generate_node_id(ctx.file_path, "interface", &name, line);
            let sym = RustSymbol {
                id: id.clone(),
                kind: "interface",
                name,
                qualified_name: qname,
                parent_id,
                file_path: ctx.file_path.to_string(),
                language: ctx.language,
                start_line: line,
                end_line,
                signature: None,
                visibility: Some(if is_exported { "public" } else { "private" }),
                is_exported,
                is_async: false,
            };
            ctx.register(sym);
            // don't descend into body for call tracking
            return;
        }
    }

    // ── structs ──────────────────────────────────────────────────────────────
    if ctx.config.struct_types.contains(&kind) {
        let line = node.start_position().row as i64 + 1;
        if let Some(name) = get_struct_name(ctx, node) {
            let end_line = node.end_position().row as i64 + 1;
            let qname = ctx.qualified_name(&name);
            let id = generate_node_id(ctx.file_path, "struct", &name, line);
            let sym = RustSymbol {
                id,
                kind: "struct",
                name,
                qualified_name: qname,
                parent_id: ctx.current_parent_id(),
                file_path: ctx.file_path.to_string(),
                language: ctx.language,
                start_line: line,
                end_line,
                signature: None,
                visibility: None,
                is_exported: false,
                is_async: false,
            };
            ctx.register(sym);
            return;
        }
    }

    // ── enums ────────────────────────────────────────────────────────────────
    if ctx.config.enum_types.contains(&kind) {
        if let Some(name) = get_named_child_text(ctx, node, "name") {
            let line = node.start_position().row as i64 + 1;
            let end_line = node.end_position().row as i64 + 1;
            let is_exported = detect_exported(ctx, node);
            let qname = ctx.qualified_name(&name);
            let id = generate_node_id(ctx.file_path, "enum", &name, line);
            let sym = RustSymbol {
                id,
                kind: "enum",
                name,
                qualified_name: qname,
                parent_id: ctx.current_parent_id(),
                file_path: ctx.file_path.to_string(),
                language: ctx.language,
                start_line: line,
                end_line,
                signature: None,
                visibility: Some(if is_exported { "public" } else { "private" }),
                is_exported,
                is_async: false,
            };
            ctx.register(sym);
            return;
        }
    }

    // ── type aliases ─────────────────────────────────────────────────────────
    if ctx.config.type_alias_types.contains(&kind) {
        if let Some((sym_kind, name)) = get_type_alias(ctx, node) {
            let line = node.start_position().row as i64 + 1;
            let end_line = node.end_position().row as i64 + 1;
            let is_exported = detect_exported(ctx, node);
            let qname = ctx.qualified_name(&name);
            let id = generate_node_id(ctx.file_path, sym_kind, &name, line);
            let sym = RustSymbol {
                id,
                kind: sym_kind,
                name,
                qualified_name: qname,
                parent_id: ctx.current_parent_id(),
                file_path: ctx.file_path.to_string(),
                language: ctx.language,
                start_line: line,
                end_line,
                signature: None,
                visibility: Some(if is_exported { "public" } else { "private" }),
                is_exported,
                is_async: false,
            };
            ctx.register(sym);
            return;
        }
    }

    // ── imports ──────────────────────────────────────────────────────────────
    if ctx.config.import_types.contains(&kind) {
        if let Some(name) = get_import_name(ctx, node) {
            let line = node.start_position().row as i64 + 1;
            let id = generate_node_id(ctx.file_path, "import", &name, line);
            let sym = RustSymbol {
                id,
                kind: "import",
                name: name.clone(),
                qualified_name: format!("{}::import::{}", ctx.file_path, name),
                parent_id: ctx.current_parent_id(),
                file_path: ctx.file_path.to_string(),
                language: ctx.language,
                start_line: line,
                end_line: line,
                signature: None,
                visibility: None,
                is_exported: false,
                is_async: false,
            };
            ctx.register(sym);
            return;
        }
    }

    // ── calls ────────────────────────────────────────────────────────────────
    if ctx.config.call_types.contains(&kind) {
        if let Some(callee) = get_call_target(ctx, node) {
            if let Some(source_id) = ctx.scope.last().map(|e| e.id.clone()) {
                let line = node.start_position().row as i64 + 1;
                ctx.calls.push(PendingCall {
                    source_id,
                    callee,
                    line: Some(line),
                });
            }
        }
    }

    // ── Go method_declaration → method ──────────────────────────────────────
    if ctx.language == "go" && kind == "method_declaration" {
        if let Some(name) = get_named_child_text(ctx, node, "name") {
            let line = node.start_position().row as i64 + 1;
            let end_line = node.end_position().row as i64 + 1;
            let receiver_type = get_go_receiver_type(ctx, node);
            let qname = if let Some(ref recv) = receiver_type {
                format!("{}::{}", recv, name)
            } else {
                ctx.qualified_name(&name)
            };
            let id = generate_node_id(ctx.file_path, "method", &name, line);
            let sym = RustSymbol {
                id: id.clone(),
                kind: "method",
                name,
                qualified_name: qname,
                parent_id: None,
                file_path: ctx.file_path.to_string(),
                language: ctx.language,
                start_line: line,
                end_line,
                signature: None,
                visibility: None,
                is_exported: false,
                is_async: false,
            };
            ctx.register(sym);
            ctx.push_scope(id, ctx.symbols.last().unwrap().name.clone(), "method");
            for i in 0..node.child_count() {
                visit_node(ctx, node.child(i).unwrap(), false);
            }
            ctx.pop_scope();
            return;
        }
    }

    // ── Go type_spec: check inner type for struct/interface ──────────────────
    if ctx.language == "go" && kind == "type_spec" {
        if let Some(name) = get_named_child_text(ctx, node, "name") {
            let inner_kind = node
                .child_by_field_name("type")
                .map(|n| n.kind())
                .unwrap_or("");
            let sym_kind: &'static str = match inner_kind {
                "struct_type" => "struct",
                "interface_type" => "interface",
                _ => "type_alias",
            };
            let line = node.start_position().row as i64 + 1;
            let end_line = node.end_position().row as i64 + 1;
            let qname = ctx.qualified_name(&name);
            let id = generate_node_id(ctx.file_path, sym_kind, &name, line);
            let sym = RustSymbol {
                id,
                kind: sym_kind,
                name,
                qualified_name: qname,
                parent_id: ctx.current_parent_id(),
                file_path: ctx.file_path.to_string(),
                language: ctx.language,
                start_line: line,
                end_line,
                signature: None,
                visibility: None,
                is_exported: false,
                is_async: false,
            };
            ctx.register(sym);
            return;
        }
    }

    // ── Ruby module → struct (namespace) ─────────────────────────────────────
    if ctx.language == "ruby" && kind == "module" {
        if let Some(name) = get_named_child_text(ctx, node, "name") {
            let line = node.start_position().row as i64 + 1;
            let end_line = node.end_position().row as i64 + 1;
            let qname = ctx.qualified_name(&name);
            let id = generate_node_id(ctx.file_path, "struct", &name, line);
            let sym = RustSymbol {
                id: id.clone(),
                kind: "struct",
                name,
                qualified_name: qname,
                parent_id: ctx.current_parent_id(),
                file_path: ctx.file_path.to_string(),
                language: ctx.language,
                start_line: line,
                end_line,
                signature: None,
                visibility: None,
                is_exported: false,
                is_async: false,
            };
            ctx.register(sym);
            ctx.push_scope(id, ctx.symbols.last().unwrap().name.clone(), "struct");
            for i in 0..node.child_count() {
                visit_node(ctx, node.child(i).unwrap(), false);
            }
            ctx.pop_scope();
            return;
        }
    }

    // ── default: recurse ─────────────────────────────────────────────────────
    let child_in_class = in_class
        || ctx
            .current_scope_kind()
            .map(|k| k == "class")
            .unwrap_or(false);
    for i in 0..node.child_count() {
        visit_node(ctx, node.child(i).unwrap(), child_in_class);
    }
}

// ── call resolution ───────────────────────────────────────────────────────────

fn resolve_calls(ctx: &mut ExtractCtx) {
    let calls: Vec<PendingCall> = std::mem::take(&mut ctx.calls);
    let mut seen = std::collections::HashSet::new();

    for call in calls {
        if let Some(target_id) = ctx.name_to_id.get(&call.callee).cloned() {
            let key = (call.source_id.clone(), target_id.clone());
            if seen.insert(key) {
                ctx.edges.push(RustGraphEdge {
                    source: call.source_id,
                    target: target_id,
                    kind: "calls",
                    line: call.line,
                });
            }
        } else {
            // Could not resolve within this file; save for cross-file resolution
            ctx.unresolved_calls
                .push((call.source_id, call.callee, call.line));
        }
    }
}

// ── name extraction helpers ───────────────────────────────────────────────────

fn get_named_child_text<'a>(ctx: &ExtractCtx<'a>, node: Node, field: &str) -> Option<String> {
    let child = node.child_by_field_name(field)?;
    let text = child.utf8_text(ctx.source).ok()?;
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn extract_function_name(ctx: &ExtractCtx, node: Node) -> Option<String> {
    // Named function: function_declaration has a "name" field
    if let Some(name) = node.child_by_field_name("name") {
        let text = ctx.node_text(name);
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    // Variable assigned arrow function or function expression: walk up to variable_declarator
    if let Some(parent) = node.parent() {
        if parent.kind() == "variable_declarator" {
            if let Some(name_node) = parent.child_by_field_name("name") {
                let text = ctx.node_text(name_node);
                if !text.is_empty() {
                    return Some(text.to_string());
                }
            }
        }
    }
    // C/C++ function: unwrap declarator chain
    if ctx.language == "c" || ctx.language == "cpp" {
        return get_cpp_function_name(ctx, node);
    }
    None
}

fn extract_method_name(ctx: &ExtractCtx, node: Node) -> Option<String> {
    // TypeScript/JS: method_definition has "name" field
    if let Some(name) = node.child_by_field_name("name") {
        let text = ctx.node_text(name);
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    // Java/C#: method_declaration has "name" field
    get_named_child_text(ctx, node, "name")
}

fn get_struct_name(ctx: &ExtractCtx, node: Node) -> Option<String> {
    // C/C++: struct_specifier / C#: struct_declaration
    if let Some(name) = node.child_by_field_name("name") {
        let text = ctx.node_text(name);
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    // Ruby module: same as struct
    None
}

fn get_type_alias(ctx: &ExtractCtx, node: Node) -> Option<(&'static str, String)> {
    // TypeScript type_alias_declaration
    if let Some(name) = node.child_by_field_name("name") {
        let text = ctx.node_text(name);
        if !text.is_empty() {
            return Some(("type_alias", text.to_string()));
        }
    }
    // C type_definition: typedef int MyInt;
    if node.kind() == "type_definition" {
        // last named child is usually the alias name
        let count = node.child_count();
        for i in (0..count).rev() {
            let child = node.child(i)?;
            if child.is_named() && child.kind() == "type_identifier" {
                let text = ctx.node_text(child);
                if !text.is_empty() {
                    return Some(("type_alias", text.to_string()));
                }
            }
        }
    }
    None
}

fn get_import_name(ctx: &ExtractCtx, node: Node) -> Option<String> {
    let kind = node.kind();
    match ctx.language {
        "typescript" | "tsx" | "javascript" | "jsx" => {
            // import_statement: source is a string_fragment child of a string
            if let Some(source) = node.child_by_field_name("source") {
                let text = ctx.node_text(source);
                let trimmed = text.trim_matches(|c| c == '"' || c == '\'');
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
            None
        }
        "python" => {
            // import_statement: module_name; import_from_statement: module_name
            if kind == "import_from_statement" {
                if let Some(module) = node.child_by_field_name("module_name") {
                    let text = ctx.node_text(module);
                    return Some(text.to_string());
                }
            }
            if kind == "import_statement" {
                if let Some(name) = node.child_by_field_name("name") {
                    let text = ctx.node_text(name);
                    return Some(text.to_string());
                }
            }
            None
        }
        "go" => {
            // import_declaration contains import_spec children with "path" field
            let mut result = None;
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "import_spec" {
                        if let Some(path) = child.child_by_field_name("path") {
                            let text = ctx.node_text(path);
                            let trimmed = text.trim_matches('"');
                            if !trimmed.is_empty() {
                                result = Some(trimmed.to_string());
                            }
                        }
                    }
                }
            }
            result
        }
        "java" => {
            // import_declaration: dotted_identifier
            let text = ctx.node_text(node);
            let clean = text.trim_start_matches("import ").trim_end_matches(';').trim();
            if !clean.is_empty() {
                return Some(clean.to_string());
            }
            None
        }
        "c" | "cpp" => {
            // preproc_include: "path" or <path>
            if let Some(path) = node.child_by_field_name("path") {
                let text = ctx.node_text(path);
                let trimmed = text.trim_matches(|c| c == '"' || c == '<' || c == '>');
                return Some(trimmed.to_string());
            }
            None
        }
        "csharp" => {
            // using_directive: name field
            if let Some(name) = node.child_by_field_name("name") {
                let text = ctx.node_text(name);
                return Some(text.to_string());
            }
            None
        }
        "ruby" => {
            // call nodes for require/require_relative
            if kind == "call" {
                let method = node.child_by_field_name("method").map(|n| ctx.node_text(n));
                if matches!(method.as_deref(), Some("require") | Some("require_relative")) {
                    if let Some(args) = node.child_by_field_name("arguments") {
                        for i in 0..args.child_count() {
                            if let Some(arg) = args.child(i) {
                                if arg.kind() == "string" || arg.kind() == "string_content" {
                                    let text = ctx.node_text(arg);
                                    let trimmed = text.trim_matches(|c| c == '"' || c == '\'');
                                    return Some(trimmed.to_string());
                                }
                            }
                        }
                    }
                }
            }
            None
        }
        "php" => {
            // include_expression / require_expression: expression child
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.is_named() {
                        let text = ctx.node_text(child);
                        let trimmed = text.trim_matches(|c| c == '"' || c == '\'');
                        if !trimmed.is_empty() {
                            return Some(trimmed.to_string());
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn get_call_target(ctx: &ExtractCtx, node: Node) -> Option<String> {
    let kind = node.kind();
    match ctx.language {
        "typescript" | "tsx" | "javascript" | "jsx" | "go" | "c" | "cpp" => {
            // call_expression: function field
            if let Some(func) = node.child_by_field_name("function") {
                let text = ctx.node_text(func);
                // strip receiver: foo.bar() → bar
                let name = text.rsplit('.').next().unwrap_or(text);
                if !name.is_empty() && is_valid_identifier(name) {
                    return Some(name.to_string());
                }
            }
            None
        }
        "python" => {
            // call: function field
            if let Some(func) = node.child_by_field_name("function") {
                let text = ctx.node_text(func);
                let name = text.rsplit('.').next().unwrap_or(text);
                if is_valid_identifier(name) {
                    return Some(name.to_string());
                }
            }
            None
        }
        "java" => {
            // method_invocation: "name" field
            if let Some(name) = node.child_by_field_name("name") {
                let text = ctx.node_text(name);
                if is_valid_identifier(text) {
                    return Some(text.to_string());
                }
            }
            None
        }
        "csharp" => {
            // invocation_expression: function field or member_access_expression
            if let Some(func) = node.child_by_field_name("function") {
                let text = ctx.node_text(func);
                let name = text.rsplit('.').next().unwrap_or(text);
                if is_valid_identifier(name) {
                    return Some(name.to_string());
                }
            }
            None
        }
        "php" => {
            // function_call_expression: function field
            if let Some(func) = node.child_by_field_name("function") {
                let text = ctx.node_text(func);
                if is_valid_identifier(text) {
                    return Some(text.to_string());
                }
            }
            None
        }
        "ruby" => {
            if kind == "call" {
                // Only non-require calls
                let method = node.child_by_field_name("method").map(|n| ctx.node_text(n));
                match method.as_deref() {
                    Some("require") | Some("require_relative") => return None,
                    Some(name) if is_valid_identifier(name) => return Some(name.to_string()),
                    _ => {}
                }
            }
            None
        }
        _ => None,
    }
}

// ── language-specific helpers ─────────────────────────────────────────────────

fn detect_exported(ctx: &ExtractCtx, node: Node) -> bool {
    match ctx.language {
        "typescript" | "tsx" | "javascript" | "jsx" => {
            // Check if parent is export_statement
            if let Some(parent) = node.parent() {
                matches!(
                    parent.kind(),
                    "export_statement" | "export_default_declaration"
                )
            } else {
                false
            }
        }
        "java" | "csharp" => {
            // Check for "public" in modifiers
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "modifiers" || child.kind() == "modifier" {
                        let text = ctx.node_text(child);
                        if text.contains("public") {
                            return true;
                        }
                    }
                }
            }
            false
        }
        "go" => {
            // Go: exported if name starts with uppercase
            if let Some(name) = node.child_by_field_name("name") {
                let text = ctx.node_text(name);
                text.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
            } else {
                false
            }
        }
        _ => false,
    }
}

fn detect_async(ctx: &ExtractCtx, node: Node) -> bool {
    match ctx.language {
        "typescript" | "tsx" | "javascript" | "jsx" | "python" | "csharp" => {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if ctx.node_text(child) == "async" {
                        return true;
                    }
                }
            }
            false
        }
        _ => false,
    }
}

fn detect_visibility(ctx: &ExtractCtx, node: Node) -> Option<&'static str> {
    match ctx.language {
        "java" | "csharp" => {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    let text = ctx.node_text(child);
                    if text.contains("public") {
                        return Some("public");
                    }
                    if text.contains("private") {
                        return Some("private");
                    }
                    if text.contains("protected") {
                        return Some("protected");
                    }
                }
            }
            None
        }
        "typescript" | "tsx" => {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    let text = ctx.node_text(child);
                    match text {
                        "public" => return Some("public"),
                        "private" => return Some("private"),
                        "protected" => return Some("protected"),
                        _ => {}
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn get_go_receiver_type(ctx: &ExtractCtx, node: Node) -> Option<String> {
    // method_declaration has a "receiver" field: parameter_list → parameter_declaration → type
    if let Some(receiver) = node.child_by_field_name("receiver") {
        for i in 0..receiver.child_count() {
            if let Some(param) = receiver.child(i) {
                if param.kind() == "parameter_declaration" {
                    if let Some(type_node) = param.child_by_field_name("type") {
                        let text = ctx.node_text(type_node);
                        // strip pointer: *Foo → Foo
                        let name = text.trim_start_matches('*');
                        return Some(name.to_string());
                    }
                }
            }
        }
    }
    None
}

fn get_cpp_function_name(ctx: &ExtractCtx, node: Node) -> Option<String> {
    // function_definition: declarator field → function_declarator → declarator → identifier
    let mut decl = node.child_by_field_name("declarator")?;
    loop {
        match decl.kind() {
            "identifier" | "field_identifier" => {
                let text = ctx.node_text(decl);
                return if text.is_empty() { None } else { Some(text.to_string()) };
            }
            "function_declarator" | "pointer_declarator" | "reference_declarator"
            | "abstract_function_declarator" => {
                if let Some(inner) = decl.child_by_field_name("declarator") {
                    decl = inner;
                } else if let Some(first) = decl.child(0) {
                    decl = first;
                } else {
                    return None;
                }
            }
            "qualified_identifier" => {
                // Foo::bar → bar
                if let Some(name) = decl.child_by_field_name("name") {
                    let text = ctx.node_text(name);
                    return if text.is_empty() { None } else { Some(text.to_string()) };
                }
                return None;
            }
            _ => return None,
        }
    }
}

fn is_valid_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .next()
            .map(|c| c.is_alphabetic() || c == '_')
            .unwrap_or(false)
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}
