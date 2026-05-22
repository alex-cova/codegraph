use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    File,
    Module,
    Class,
    Struct,
    Interface,
    Trait,
    Protocol,
    Function,
    Method,
    Property,
    Field,
    Variable,
    Constant,
    Enum,
    EnumMember,
    TypeAlias,
    Namespace,
    Parameter,
    Import,
    Export,
    Route,
    Component,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Typescript,
    Javascript,
    Tsx,
    Jsx,
    Python,
    Go,
    Rust,
    Java,
    C,
    Cpp,
    Csharp,
    Php,
    Ruby,
    Swift,
    Kotlin,
    Dart,
    Svelte,
    Vue,
    Liquid,
    Pascal,
    Scala,
    Lua,
    Luau,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Contains,
    Calls,
    Imports,
    Exports,
    Extends,
    Implements,
    References,
    TypeOf,
    Returns,
    Instantiates,
    Overrides,
    Decorates,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FrameworkPatterns {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameworkHint {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patterns: Option<FrameworkPatterns>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomPattern {
    pub name: String,
    pub pattern: String,
    pub kind: NodeKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeGraphConfig {
    pub version: u32,
    pub root_dir: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub languages: Vec<Language>,
    pub frameworks: Vec<FrameworkHint>,
    pub max_file_size: u64,
    pub extract_docstrings: bool,
    pub track_call_sites: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_patterns: Option<Vec<CustomPattern>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    pub name: String,
    pub qualified_name: String,
    pub file_path: String,
    pub language: Language,
    pub start_line: i64,
    pub end_line: i64,
    pub start_column: i64,
    pub end_column: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_exported: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_async: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_static: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_abstract: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decorators: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_parameters: Option<Vec<String>>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub kind: EdgeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRecord {
    pub path: String,
    pub content_hash: String,
    pub language: Language,
    pub size: i64,
    pub modified_at: i64,
    pub indexed_at: i64,
    pub node_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GraphStats {
    pub node_count: i64,
    pub edge_count: i64,
    pub file_count: i64,
    pub nodes_by_kind: std::collections::BTreeMap<String, i64>,
    pub edges_by_kind: std::collections::BTreeMap<String, i64>,
    pub files_by_language: std::collections::BTreeMap<String, i64>,
    pub db_size_bytes: u64,
    pub last_updated: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Subgraph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub roots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeEdgeRef {
    pub node: Node,
    pub edge: Edge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    pub focal: Node,
    pub ancestors: Vec<Node>,
    pub children: Vec<Node>,
    pub incoming_refs: Vec<NodeEdgeRef>,
    pub outgoing_refs: Vec<NodeEdgeRef>,
    pub types: Vec<Node>,
    pub imports: Vec<Node>,
}

impl CodeGraphConfig {
    pub fn default_for(project_root: &str) -> Self {
        Self {
            version: 1,
            root_dir: project_root.to_string(),
            include: vec![
                "**/*.ts", "**/*.tsx", "**/*.js", "**/*.jsx", "**/*.py", "**/*.go", "**/*.rs",
                "**/*.java", "**/*.c", "**/*.h", "**/*.cpp", "**/*.hpp", "**/*.cc", "**/*.cxx",
                "**/*.cs", "**/*.php", "**/*.rb", "**/*.swift", "**/*.kt", "**/*.kts",
                "**/*.dart", "**/*.svelte", "**/*.vue", "**/*.liquid", "**/*.pas", "**/*.dpr",
                "**/*.dpk", "**/*.lpr", "**/*.dfm", "**/*.fmx", "**/*.scala", "**/*.sc",
                "**/*.lua", "**/*.luau",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            exclude: vec![
                "**/.git/**",
                "**/node_modules/**",
                "**/vendor/**",
                "**/Pods/**",
                "**/dist/**",
                "**/build/**",
                "**/out/**",
                "**/bin/**",
                "**/obj/**",
                "**/target/**",
                "**/*.min.js",
                "**/*.bundle.js",
                "**/.next/**",
                "**/.nuxt/**",
                "**/.svelte-kit/**",
                "**/.output/**",
                "**/.turbo/**",
                "**/.cache/**",
                "**/.parcel-cache/**",
                "**/.vite/**",
                "**/.astro/**",
                "**/.docusaurus/**",
                "**/.gatsby/**",
                "**/.webpack/**",
                "**/.nx/**",
                "**/.yarn/cache/**",
                "**/.pnpm-store/**",
                "**/storybook-static/**",
                "**/.expo/**",
                "**/web-build/**",
                "**/ios/Pods/**",
                "**/ios/build/**",
                "**/android/build/**",
                "**/android/.gradle/**",
                "**/__pycache__/**",
                "**/.venv/**",
                "**/venv/**",
                "**/site-packages/**",
                "**/dist-packages/**",
                "**/.pytest_cache/**",
                "**/.mypy_cache/**",
                "**/.ruff_cache/**",
                "**/.tox/**",
                "**/.nox/**",
                "**/*.egg-info/**",
                "**/.eggs/**",
                "**/go/pkg/mod/**",
                "**/target/debug/**",
                "**/target/release/**",
                "**/.gradle/**",
                "**/.m2/**",
                "**/generated-sources/**",
                "**/.kotlin/**",
                "**/.dart_tool/**",
                "**/.vs/**",
                "**/.nuget/**",
                "**/artifacts/**",
                "**/publish/**",
                "**/cmake-build-*/**",
                "**/CMakeFiles/**",
                "**/bazel-*/**",
                "**/vcpkg_installed/**",
                "**/.conan/**",
                "**/Debug/**",
                "**/Release/**",
                "**/x64/**",
                "**/.pio/**",
                "**/release/**",
                "**/*.app/**",
                "**/*.asar",
                "**/DerivedData/**",
                "**/.build/**",
                "**/.swiftpm/**",
                "**/xcuserdata/**",
                "**/Carthage/Build/**",
                "**/SourcePackages/**",
                "**/__history/**",
                "**/__recovery/**",
                "**/*.dcu",
                "**/.composer/**",
                "**/storage/framework/**",
                "**/bootstrap/cache/**",
                "**/.bundle/**",
                "**/tmp/cache/**",
                "**/public/assets/**",
                "**/public/packs/**",
                "**/.yardoc/**",
                "**/coverage/**",
                "**/htmlcov/**",
                "**/.nyc_output/**",
                "**/test-results/**",
                "**/.coverage/**",
                "**/.idea/**",
                "**/logs/**",
                "**/tmp/**",
                "**/temp/**",
                "**/_build/**",
                "**/docs/_build/**",
                "**/site/**",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            languages: Vec::new(),
            frameworks: Vec::new(),
            max_file_size: 1024 * 1024,
            extract_docstrings: true,
            track_call_sites: true,
            custom_patterns: None,
        }
    }
}
