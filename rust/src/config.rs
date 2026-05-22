use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::types::CodeGraphConfig;

pub const CONFIG_FILENAME: &str = "config.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PartialCodeGraphConfig {
    version: Option<u32>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    languages: Option<Vec<crate::types::Language>>,
    frameworks: Option<Vec<crate::types::FrameworkHint>>,
    max_file_size: Option<u64>,
    extract_docstrings: Option<bool>,
    track_call_sites: Option<bool>,
    custom_patterns: Option<Vec<crate::types::CustomPattern>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistedCodeGraphConfig<'a> {
    version: u32,
    include: &'a [String],
    exclude: &'a [String],
    languages: &'a [crate::types::Language],
    frameworks: &'a [crate::types::FrameworkHint],
    max_file_size: u64,
    extract_docstrings: bool,
    track_call_sites: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    custom_patterns: &'a Option<Vec<crate::types::CustomPattern>>,
}

pub fn get_config_path(project_root: &Path) -> PathBuf {
    project_root.join(".codegraph").join(CONFIG_FILENAME)
}

pub fn create_default_config(project_root: &Path) -> CodeGraphConfig {
    CodeGraphConfig::default_for(&project_root.to_string_lossy())
}

pub fn load_config(project_root: &Path) -> Result<CodeGraphConfig> {
    let config_path = get_config_path(project_root);
    if !config_path.exists() {
        return Ok(create_default_config(project_root));
    }

    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("failed to read {}", config_path.display()))?;
    let parsed: PartialCodeGraphConfig = serde_json::from_str(&content)
        .with_context(|| format!("invalid JSON in {}", config_path.display()))?;

    let mut merged = create_default_config(project_root);
    if let Some(version) = parsed.version {
        merged.version = version;
    }
    if let Some(include) = parsed.include {
        merged.include = include;
    }
    if let Some(exclude) = parsed.exclude {
        merged.exclude = exclude;
    }
    if let Some(languages) = parsed.languages {
        merged.languages = languages;
    }
    if let Some(frameworks) = parsed.frameworks {
        merged.frameworks = frameworks;
    }
    if let Some(max_file_size) = parsed.max_file_size {
        merged.max_file_size = max_file_size;
    }
    if let Some(extract_docstrings) = parsed.extract_docstrings {
        merged.extract_docstrings = extract_docstrings;
    }
    if let Some(track_call_sites) = parsed.track_call_sites {
        merged.track_call_sites = track_call_sites;
    }
    if let Some(custom_patterns) = parsed.custom_patterns {
        merged.custom_patterns = Some(custom_patterns);
    }

    validate_config(&merged)?;
    Ok(merged)
}

pub fn save_config(project_root: &Path, config: &CodeGraphConfig) -> Result<()> {
    validate_config(config)?;

    let config_path = get_config_path(project_root);
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let to_save = PersistedCodeGraphConfig {
        version: config.version,
        include: &config.include,
        exclude: &config.exclude,
        languages: &config.languages,
        frameworks: &config.frameworks,
        max_file_size: config.max_file_size,
        extract_docstrings: config.extract_docstrings,
        track_call_sites: config.track_call_sites,
        custom_patterns: &config.custom_patterns,
    };

    let content = serde_json::to_string_pretty(&to_save)?;
    let tmp_path = config_path.with_extension("json.tmp");
    fs::write(&tmp_path, content)
        .with_context(|| format!("failed to write {}", tmp_path.display()))?;
    fs::rename(&tmp_path, &config_path)
        .with_context(|| format!("failed to replace {}", config_path.display()))?;

    Ok(())
}

pub fn validate_config(config: &CodeGraphConfig) -> Result<()> {
    if config.version == 0 {
        bail!("config version must be greater than 0");
    }
    if config.root_dir.is_empty() {
        bail!("config root_dir must not be empty");
    }
    if config.max_file_size == 0 {
        bail!("config max_file_size must be greater than 0");
    }

    if let Some(custom_patterns) = &config.custom_patterns {
        for custom in custom_patterns {
            if custom.name.trim().is_empty() {
                bail!("custom pattern name must not be empty");
            }
            if !is_safe_regex(&custom.pattern)? {
                bail!("unsafe custom regex pattern: {}", custom.name);
            }
        }
    }

    Ok(())
}

fn is_safe_regex(pattern: &str) -> Result<bool> {
    if pattern.len() > 500 {
        return Ok(false);
    }

    let repeated_quantifiers = Regex::new(r"([+*}])\s*[+*{]")?;
    let nested_quantifiers = Regex::new(r"\([^)]*[+*][^)]*\)[+*{]")?;
    if repeated_quantifiers.is_match(pattern) || nested_quantifiers.is_match(pattern) {
        return Ok(false);
    }

    Ok(Regex::new(pattern).is_ok())
}
