use crate::extraction::{generate_node_id, RustArtifacts, RustGraphEdge, RustSymbol};
use crate::ts_extraction::extract_with_tree_sitter;

// ── public API ────────────────────────────────────────────────────────────────

/// Extract from a `.svelte` file. Returns a component node plus everything
/// from the embedded `<script>` block (parsed as TypeScript or JavaScript).
pub(crate) fn extract_svelte(file_path: &str, content: &str) -> RustArtifacts {
    extract_component(file_path, content, "svelte")
}

/// Extract from a `.vue` SFC file. Returns a component node plus everything
/// from the embedded `<script>` block.
pub(crate) fn extract_vue(file_path: &str, content: &str) -> RustArtifacts {
    extract_component(file_path, content, "vue")
}

// ── implementation ────────────────────────────────────────────────────────────

fn extract_component(file_path: &str, content: &str, format: &str) -> RustArtifacts {
    let component_name = component_name_from_path(file_path);
    let line_count = content.lines().count().max(1) as i64;

    // Build the top-level component node
    let comp_id = generate_node_id(file_path, "component", &component_name, 1);
    let comp_node = RustSymbol {
        id: comp_id.clone(),
        kind: "component",
        name: component_name.clone(),
        qualified_name: format!("{}::{}", file_path, component_name),
        parent_id: None,
        file_path: file_path.to_string(),
        language: format_lang(format),
        start_line: 1,
        end_line: line_count,
        signature: None,
        visibility: Some("public"),
        is_exported: true,
        is_async: false,
    };

    // Find and parse the <script> block
    let mut artifacts = if let Some(script) = extract_script_block(content) {
        let lang = script.lang.unwrap_or("javascript");
        let ts_lang = if lang == "ts" || lang == "typescript" {
            "typescript"
        } else {
            "javascript"
        };
        let line_offset = script.start_line;

        if let Some(mut inner) = extract_with_tree_sitter(file_path, ts_lang, &script.content) {
            // Adjust line numbers to be relative to the full file
            for sym in &mut inner.symbols {
                sym.start_line += line_offset;
                sym.end_line += line_offset;
                // Re-parent top-level symbols to the component node
                if sym.parent_id.is_none() {
                    sym.parent_id = Some(comp_id.clone());
                }
            }
            inner
        } else {
            RustArtifacts::default()
        }
    } else {
        RustArtifacts::default()
    };

    // Prepend the component node
    artifacts.symbols.insert(0, comp_node);

    // Add contains edges: component → each top-level child
    let child_ids: Vec<String> = artifacts
        .symbols
        .iter()
        .skip(1)
        .filter(|s| s.parent_id.as_deref() == Some(&comp_id))
        .map(|s| s.id.clone())
        .collect();
    for child_id in child_ids {
        artifacts.edges.push(RustGraphEdge {
            source: comp_id.clone(),
            target: child_id,
            kind: "contains",
            line: None,
        });
    }

    artifacts
}

// ── script block extraction ───────────────────────────────────────────────────

struct ScriptBlock {
    content: String,
    lang: Option<&'static str>,
    start_line: i64, // 0-indexed line where the content begins (after <script ...>)
}

/// Find the first `<script ...>` block and return its inner content + metadata.
fn extract_script_block(content: &str) -> Option<ScriptBlock> {
    // Match <script> or <script lang="ts"> or <script setup lang="ts"> etc.
    // We use a simple state machine rather than a regex to handle multi-line tags.
    let lower = content.to_ascii_lowercase();
    let script_start = lower.find("<script")?;
    let tag_end = lower[script_start..].find('>')?;
    let tag_text = &content[script_start..script_start + tag_end + 1];

    // Detect lang attribute
    let lang: Option<&'static str> = if tag_text.to_ascii_lowercase().contains("lang=\"ts\"")
        || tag_text.to_ascii_lowercase().contains("lang='ts'")
    {
        Some("ts")
    } else {
        None
    };

    let body_start = script_start + tag_end + 1;
    let close_tag = lower[body_start..].find("</script>")?;
    let body = &content[body_start..body_start + close_tag];

    // Count lines before body_start to get the line offset
    let start_line = content[..body_start]
        .chars()
        .filter(|&c| c == '\n')
        .count() as i64;

    Some(ScriptBlock {
        content: body.to_string(),
        lang,
        start_line,
    })
}

fn component_name_from_path(file_path: &str) -> String {
    Path::new(file_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Component")
        .to_string()
}

fn format_lang(format: &str) -> &'static str {
    match format {
        "svelte" => "svelte",
        "vue" => "vue",
        _ => "unknown",
    }
}

use std::path::Path;

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_svelte_component_node() {
        let content = r#"
<script lang="ts">
  export function greet(name: string): string {
    return `Hello ${name}`;
  }
</script>

<template>
  <h1>Hello</h1>
</template>
"#;
        let artifacts = extract_svelte("src/Greeting.svelte", content);
        let kinds: Vec<&str> = artifacts.symbols.iter().map(|s| s.kind).collect();
        let names: Vec<&str> = artifacts.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(kinds.contains(&"component"), "expected component node");
        assert!(kinds.contains(&"function"), "expected function node");
        assert!(names.contains(&"Greeting"), "expected Greeting component");
        assert!(names.contains(&"greet"), "expected greet function");
    }

    #[test]
    fn extracts_vue_component_node() {
        let content = r#"
<script>
export default {
  name: 'MyComponent',
};

export function helper() { return 42; }
</script>

<template><div /></template>
"#;
        let artifacts = extract_vue("src/MyComponent.vue", content);
        let kinds: Vec<&str> = artifacts.symbols.iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&"component"), "expected component node");
    }

    #[test]
    fn svelte_with_no_script_block_has_component_only() {
        let content = "<template><div>Hello</div></template>\n";
        let artifacts = extract_svelte("src/Static.svelte", content);
        assert_eq!(artifacts.symbols.len(), 1);
        assert_eq!(artifacts.symbols[0].kind, "component");
    }
}
