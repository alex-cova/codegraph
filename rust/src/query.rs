use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params_from_iter};
use serde::de::DeserializeOwned;

use crate::search::{
    bounded_edit_distance, extract_search_terms, kind_bonus, name_match_bonus, parse_query,
    score_path_relevance,
};
use crate::db::{self, DatabaseInfo};
use crate::types::{Edge, EdgeKind, FileRecord, GraphStats, Language, Node, NodeKind};

pub struct QueryService {
    conn: Connection,
    db_info: DatabaseInfo,
}

impl QueryService {
    pub fn open(project_root: &Path) -> Result<Self> {
        let db_info = db::open_database(project_root)?;
        let conn = Connection::open(&db_info.path)
            .with_context(|| format!("failed to open {}", db_info.path.display()))?;
        Ok(Self { conn, db_info })
    }

    pub fn database_info(&self) -> &DatabaseInfo {
        &self.db_info
    }

    pub fn get_stats(&self) -> Result<GraphStats> {
        let counts = self.conn.query_row(
            "SELECT
                (SELECT COUNT(*) FROM nodes) AS node_count,
                (SELECT COUNT(*) FROM edges) AS edge_count,
                (SELECT COUNT(*) FROM files) AS file_count",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )?;

        Ok(GraphStats {
            node_count: counts.0,
            edge_count: counts.1,
            file_count: counts.2,
            nodes_by_kind: self.group_counts("SELECT kind, COUNT(*) FROM nodes GROUP BY kind")?,
            edges_by_kind: self.group_counts("SELECT kind, COUNT(*) FROM edges GROUP BY kind")?,
            files_by_language: self
                .group_counts("SELECT language, COUNT(*) FROM files GROUP BY language")?,
            db_size_bytes: self.db_info.size_bytes,
            last_updated: unix_time_ms(),
        })
    }

    pub fn get_node_by_id(&self, id: &str) -> Result<Option<Node>> {
        self.conn
            .query_row("SELECT * FROM nodes WHERE id = ?1", [id], row_to_node)
            .optional()
            .context("failed to fetch node by id")
    }

    pub fn get_nodes_by_file(&self, file_path: &str) -> Result<Vec<Node>> {
        let mut stmt = self
            .conn
            .prepare("SELECT * FROM nodes WHERE file_path = ?1 ORDER BY start_line")?;
        let rows = stmt.query_map([file_path], row_to_node)?;
        collect_rows(rows)
    }

    pub fn get_nodes_by_kind(&self, kind: NodeKind) -> Result<Vec<Node>> {
        let raw = enum_to_db_value(&kind)?;
        let mut stmt = self.conn.prepare("SELECT * FROM nodes WHERE kind = ?1")?;
        let rows = stmt.query_map([raw], row_to_node)?;
        collect_rows(rows)
    }

    pub fn get_all_files(&self) -> Result<Vec<FileRecord>> {
        let mut stmt = self.conn.prepare("SELECT * FROM files ORDER BY path")?;
        let rows = stmt.query_map([], row_to_file_record)?;
        collect_rows(rows)
    }

    pub fn get_outgoing_edges(
        &self,
        source_id: &str,
        kinds: Option<&[EdgeKind]>,
    ) -> Result<Vec<Edge>> {
        let mut sql = String::from("SELECT * FROM edges WHERE source = ?");
        let mut params: Vec<String> = vec![source_id.to_string()];
        if let Some(kinds) = kinds {
            if !kinds.is_empty() {
                sql.push_str(" AND kind IN (");
                sql.push_str(&vec!["?"; kinds.len()].join(","));
                sql.push(')');
                for kind in kinds {
                    params.push(enum_to_db_value(kind)?);
                }
            }
        }
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.iter()), row_to_edge)?;
        collect_rows(rows)
    }

    pub fn get_incoming_edges(
        &self,
        target_id: &str,
        kinds: Option<&[EdgeKind]>,
    ) -> Result<Vec<Edge>> {
        let mut sql = String::from("SELECT * FROM edges WHERE target = ?");
        let mut params: Vec<String> = vec![target_id.to_string()];
        if let Some(kinds) = kinds {
            if !kinds.is_empty() {
                sql.push_str(" AND kind IN (");
                sql.push_str(&vec!["?"; kinds.len()].join(","));
                sql.push(')');
                for kind in kinds {
                    params.push(enum_to_db_value(kind)?);
                }
            }
        }
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.iter()), row_to_edge)?;
        collect_rows(rows)
    }

    pub fn search_nodes(
        &self,
        query: &str,
        kind: Option<&str>,
        language: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Node>> {
        let parsed = parse_query(query);
        let mut kinds = parsed.kinds.clone();
        if let Some(kind) = kind {
            kinds.push(kind.to_string());
        }
        kinds.sort();
        kinds.dedup();

        let mut languages = parsed.languages.clone();
        if let Some(language) = language {
            languages.push(language.to_string());
        }
        languages.sort();
        languages.dedup();

        let search_text = parsed.text.trim();
        let mut scored = if !search_text.is_empty() {
            let mut results = self.search_nodes_fts(search_text, &kinds, &languages, limit)?;
            if results.is_empty() && search_text.len() >= 2 {
                results = self.search_nodes_like(search_text, &kinds, &languages, limit)?;
            }
            if results.is_empty() && search_text.len() >= 3 {
                results = self.search_nodes_fuzzy(search_text, &kinds, &languages, limit)?;
            }
            results
        } else {
            self.search_all_by_filters(&kinds, &languages, limit.saturating_mul(5).max(limit))?
        };

        let exact_terms: Vec<String> = if search_text.is_empty() {
            Vec::new()
        } else {
            search_text
                .split_whitespace()
                .filter(|term| term.len() >= 2)
                .map(|term| term.to_string())
                .collect()
        };
        self.supplement_exact_name_matches(&mut scored, &exact_terms, &kinds, &languages)?;

        let scoring_query = if !search_text.is_empty() { search_text } else { query };
        for item in &mut scored {
            item.score += kind_bonus(&item.node.kind) as f64;
            item.score += score_path_relevance(&item.node.file_path, scoring_query) as f64;
            item.score += name_match_bonus(&item.node.name, scoring_query) as f64;
        }
        scored.sort_by(|a, b| b.score.total_cmp(&a.score));

        if !parsed.path_filters.is_empty() {
            let lowered: Vec<String> = parsed
                .path_filters
                .iter()
                .map(|item| item.to_lowercase())
                .collect();
            scored.retain(|item| {
                let file_path = item.node.file_path.to_lowercase();
                lowered.iter().any(|path| file_path.contains(path))
            });
        }
        if !parsed.name_filters.is_empty() {
            let lowered: Vec<String> = parsed
                .name_filters
                .iter()
                .map(|item| item.to_lowercase())
                .collect();
            scored.retain(|item| {
                let name = item.node.name.to_lowercase();
                lowered.iter().any(|needle| name.contains(needle))
            });
        }

        if scored.len() > limit {
            scored.truncate(limit);
        }

        Ok(scored.into_iter().map(|item| item.node).collect())
    }

    fn group_counts(&self, sql: &str) -> Result<BTreeMap<String, i64>> {
        let mut stmt = self.conn.prepare(sql)?;
        let mut rows = stmt.query([])?;
        let mut counts = BTreeMap::new();
        while let Some(row) = rows.next()? {
            counts.insert(row.get::<_, String>(0)?, row.get::<_, i64>(1)?);
        }
        Ok(counts)
    }

    fn search_all_by_filters(
        &self,
        kinds: &[String],
        languages: &[String],
        limit: usize,
    ) -> Result<Vec<ScoredNode>> {
        let limit_i64 = i64::try_from(limit).context("query limit out of range")?;
        let mut sql = String::from("SELECT * FROM nodes WHERE 1=1");
        let mut params: Vec<String> = Vec::new();
        append_string_filters(&mut sql, &mut params, "kind", kinds);
        append_string_filters(&mut sql, &mut params, "language", languages);
        sql.push_str(" ORDER BY name LIMIT ?");
        params.push(limit_i64.to_string());

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.iter()), row_to_node)?;
        Ok(collect_rows(rows)?
            .into_iter()
            .map(|node| ScoredNode { node, score: 1.0 })
            .collect())
    }

    fn search_nodes_fts(
        &self,
        query: &str,
        kinds: &[String],
        languages: &[String],
        limit: usize,
    ) -> Result<Vec<ScoredNode>> {
        let fts_query = build_fts_query(query);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }

        let fts_limit = i64::try_from((limit.saturating_mul(5)).max(100))
            .context("query limit out of range")?;
        let mut sql = String::from(
            "SELECT nodes.*, bm25(nodes_fts, 0, 20, 5, 1, 2) as score
             FROM nodes_fts
             JOIN nodes ON nodes_fts.id = nodes.id
             WHERE nodes_fts MATCH ?",
        );
        let mut params = vec![fts_query];
        append_string_filters_prefixed(&mut sql, &mut params, "nodes.kind", kinds);
        append_string_filters_prefixed(&mut sql, &mut params, "nodes.language", languages);
        sql.push_str(" ORDER BY score LIMIT ?");
        params.push(fts_limit.to_string());

        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(params_from_iter(params.iter()))?;
        let mut results = Vec::new();
        while let Some(row) = rows.next()? {
            let score: f64 = row.get("score")?;
            results.push(ScoredNode {
                node: row_to_node(row)?,
                score: score.abs(),
            });
        }
        Ok(results)
    }

    fn search_nodes_like(
        &self,
        query: &str,
        kinds: &[String],
        languages: &[String],
        limit: usize,
    ) -> Result<Vec<ScoredNode>> {
        let limit_i64 = i64::try_from(limit).context("query limit out of range")?;
        let exact_match = query.to_string();
        let starts_with = format!("{query}%");
        let contains = format!("%{query}%");
        let mut sql = String::from(
            "SELECT nodes.*,
                CASE
                    WHEN name = ?1 THEN 1.0
                    WHEN name LIKE ?2 THEN 0.9
                    WHEN name LIKE ?3 THEN 0.8
                    WHEN qualified_name LIKE ?4 THEN 0.7
                    ELSE 0.5
                END as score
             FROM nodes
             WHERE (name LIKE ?5 OR qualified_name LIKE ?6 OR name LIKE ?7)",
        );
        let mut params = vec![
            exact_match,
            starts_with.clone(),
            contains.clone(),
            contains.clone(),
            contains.clone(),
            contains,
            starts_with,
        ];
        append_string_filters(&mut sql, &mut params, "kind", kinds);
        append_string_filters(&mut sql, &mut params, "language", languages);
        sql.push_str(" ORDER BY score DESC, length(name) ASC LIMIT ?");
        params.push(limit_i64.to_string());

        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(params_from_iter(params.iter()))?;
        let mut results = Vec::new();
        while let Some(row) = rows.next()? {
            let score: f64 = row.get("score")?;
            results.push(ScoredNode {
                node: row_to_node(row)?,
                score,
            });
        }
        Ok(results)
    }

    fn search_nodes_fuzzy(
        &self,
        text: &str,
        kinds: &[String],
        languages: &[String],
        limit: usize,
    ) -> Result<Vec<ScoredNode>> {
        let lowered = text.to_lowercase();
        let max_dist = if lowered.len() <= 4 { 1 } else { 2 };
        let all_names = self.get_all_node_names()?;
        let mut candidates = Vec::new();
        for name in all_names {
            let distance = bounded_edit_distance(&name.to_lowercase(), &lowered, max_dist);
            if distance <= max_dist {
                candidates.push((name, distance));
            }
        }
        candidates.sort_by(|a, b| a.1.cmp(&b.1));

        let follow_up_cap = (limit.saturating_mul(2)).max(50);
        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (name, distance) in candidates.into_iter().take(follow_up_cap) {
            if results.len() >= limit {
                break;
            }
            let mut sql = String::from("SELECT * FROM nodes WHERE name = ?");
            let mut params = vec![name];
            append_string_filters(&mut sql, &mut params, "kind", kinds);
            append_string_filters(&mut sql, &mut params, "language", languages);
            sql.push_str(" LIMIT 5");

            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(params.iter()), row_to_node)?;
            for node in collect_rows(rows)? {
                if seen.insert(node.id.clone()) {
                    results.push(ScoredNode {
                        node,
                        score: 1.0 / (1.0 + distance as f64),
                    });
                    if results.len() >= limit {
                        break;
                    }
                }
            }
        }
        Ok(results)
    }

    fn supplement_exact_name_matches(
        &self,
        results: &mut Vec<ScoredNode>,
        terms: &[String],
        kinds: &[String],
        languages: &[String],
    ) -> Result<()> {
        if results.is_empty() || terms.is_empty() {
            return Ok(());
        }

        let mut seen_ids: std::collections::HashSet<String> =
            results.iter().map(|item| item.node.id.clone()).collect();
        let max_score = results
            .iter()
            .map(|item| item.score)
            .fold(0.0f64, f64::max);

        for term in terms {
            let mut sql = String::from("SELECT * FROM nodes WHERE name = ? COLLATE NOCASE");
            let mut params = vec![term.clone()];
            append_string_filters(&mut sql, &mut params, "kind", kinds);
            append_string_filters(&mut sql, &mut params, "language", languages);
            sql.push_str(" LIMIT 20");

            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(params.iter()), row_to_node)?;
            for node in collect_rows(rows)? {
                if seen_ids.insert(node.id.clone()) {
                    results.push(ScoredNode {
                        node,
                        score: max_score,
                    });
                }
            }
        }

        Ok(())
    }

    fn get_all_node_names(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT name FROM nodes ORDER BY name")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        collect_rows(rows)
    }

    pub fn get_all_nodes(&self, limit: usize) -> Result<Vec<Node>> {
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut stmt = self
            .conn
            .prepare("SELECT * FROM nodes ORDER BY kind, name LIMIT ?")?;
        let rows = stmt.query_map([limit_i64], row_to_node)?;
        collect_rows(rows)
    }

    pub fn get_all_edges(&self, limit: usize) -> Result<Vec<Edge>> {
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut stmt = self
            .conn
            .prepare("SELECT * FROM edges LIMIT ?")?;
        let rows = stmt.query_map([limit_i64], row_to_edge)?;
        collect_rows(rows)
    }

    pub fn get_unused_no_inbound_ids(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM nodes \
             WHERE kind NOT IN ('file','import','export','parameter','enum_member') \
             AND NOT EXISTS ( \
               SELECT 1 FROM edges \
               WHERE target = nodes.id \
               AND kind IN ('calls','references','imports','instantiates','decorates') \
             )",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        collect_rows(rows)
    }

    pub fn get_unused_unexported_unreferenced_ids(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM nodes \
             WHERE (is_exported = 0 OR is_exported IS NULL) \
             AND kind NOT IN ('file','import','export','parameter','enum_member') \
             AND NOT EXISTS (SELECT 1 FROM edges WHERE target = nodes.id)",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        collect_rows(rows)
    }

    pub fn get_orphan_file_ids(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM nodes WHERE kind = 'file' \
             AND NOT EXISTS (SELECT 1 FROM edges WHERE source = nodes.id AND kind = 'imports') \
             AND NOT EXISTS (SELECT 1 FROM edges WHERE target = nodes.id AND kind = 'imports')",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        collect_rows(rows)
    }

    pub fn get_dead_route_ids(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM nodes WHERE kind = 'route' \
             AND NOT EXISTS (SELECT 1 FROM edges WHERE target = nodes.id)",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        collect_rows(rows)
    }
}

#[derive(Debug, Clone)]
struct ScoredNode {
    node: Node,
    score: f64,
}

fn row_to_node(row: &rusqlite::Row<'_>) -> rusqlite::Result<Node> {
    Ok(Node {
        id: row.get("id")?,
        kind: parse_db_enum::<NodeKind>(&row.get::<_, String>("kind")?)?,
        name: row.get("name")?,
        qualified_name: row.get("qualified_name")?,
        file_path: row.get("file_path")?,
        language: parse_db_enum::<Language>(&row.get::<_, String>("language")?)?,
        start_line: row.get("start_line")?,
        end_line: row.get("end_line")?,
        start_column: row.get("start_column")?,
        end_column: row.get("end_column")?,
        docstring: row.get("docstring")?,
        signature: row.get("signature")?,
        visibility: row.get("visibility")?,
        is_exported: Some(row.get::<_, i64>("is_exported")? == 1),
        is_async: Some(row.get::<_, i64>("is_async")? == 1),
        is_static: Some(row.get::<_, i64>("is_static")? == 1),
        is_abstract: Some(row.get::<_, i64>("is_abstract")? == 1),
        decorators: parse_optional_json_array(row.get("decorators")?),
        type_parameters: parse_optional_json_array(row.get("type_parameters")?),
        updated_at: row.get("updated_at")?,
    })
}

fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<Edge> {
    Ok(Edge {
        source: row.get("source")?,
        target: row.get("target")?,
        kind: parse_db_enum::<EdgeKind>(&row.get::<_, String>("kind")?)?,
        metadata: parse_optional_json_value(row.get("metadata")?),
        line: row.get("line")?,
        column: row.get("col")?,
        provenance: row.get("provenance")?,
    })
}

fn row_to_file_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<FileRecord> {
    Ok(FileRecord {
        path: row.get("path")?,
        content_hash: row.get("content_hash")?,
        language: parse_db_enum::<Language>(&row.get::<_, String>("language")?)?,
        size: row.get("size")?,
        modified_at: row.get("modified_at")?,
        indexed_at: row.get("indexed_at")?,
        node_count: row.get("node_count")?,
        errors: parse_optional_json_value(row.get("errors")?),
    })
}

fn collect_rows<T, F>(rows: rusqlite::MappedRows<'_, F>) -> Result<Vec<T>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut items = Vec::new();
    for row in rows {
        items.push(row?);
    }
    Ok(items)
}

fn parse_db_enum<T: DeserializeOwned>(raw: &str) -> rusqlite::Result<T> {
    serde_json::from_str(&format!("\"{raw}\""))
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err)))
}

fn enum_to_db_value<T: serde::Serialize>(value: &T) -> Result<String> {
    let encoded = serde_json::to_string(value)?;
    Ok(encoded.trim_matches('"').to_string())
}

fn parse_optional_json_array(raw: Option<String>) -> Option<Vec<String>> {
    raw.and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
}

fn parse_optional_json_value(raw: Option<String>) -> Option<serde_json::Value> {
    raw.and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok())
}

fn build_fts_query(query: &str) -> String {
    let base_terms = query
        .replace("::", " ")
        .chars()
        .map(|c| match c {
            '\'' | '"' | '*' | '(' | ')' | ':' | '^' => ' ',
            _ => c,
        })
        .collect::<String>();
    let mut terms: Vec<String> = base_terms
        .split_whitespace()
        .filter(|term| !matches!((*term).to_ascii_uppercase().as_str(), "AND" | "OR" | "NOT" | "NEAR"))
        .map(|term| format!("\"{term}\"*"))
        .collect();

    for term in extract_search_terms(query, true) {
        if term.len() >= 2 {
            terms.push(format!("\"{term}\"*"));
        }
    }
    terms.sort();
    terms.dedup();
    terms.join(" OR ")
}

fn append_string_filters(sql: &mut String, params: &mut Vec<String>, column: &str, values: &[String]) {
    append_string_filters_prefixed(sql, params, column, values);
}

fn append_string_filters_prefixed(
    sql: &mut String,
    params: &mut Vec<String>,
    column: &str,
    values: &[String],
) {
    if values.is_empty() {
        return;
    }
    sql.push_str(&format!(" AND {column} IN ({})", vec!["?"; values.len()].join(",")));
    params.extend(values.iter().cloned());
}

fn unix_time_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
