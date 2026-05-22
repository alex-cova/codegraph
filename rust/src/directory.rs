use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

pub const CODEGRAPH_DIR: &str = ".codegraph";

const GITIGNORE_CONTENT: &str = "# CodeGraph data files
# These are local to each machine and should not be committed

# Database
*.db
*.db-wal
*.db-shm

# Cache
cache/

# Logs
*.log

# Hook markers
.dirty
";

pub fn get_codegraph_dir(project_root: &Path) -> PathBuf {
    project_root.join(CODEGRAPH_DIR)
}

pub fn is_initialized(project_root: &Path) -> bool {
    let codegraph_dir = get_codegraph_dir(project_root);
    if !codegraph_dir.is_dir() {
        return false;
    }
    codegraph_dir.join("codegraph.db").exists()
}

pub fn create_directory(project_root: &Path) -> Result<()> {
    let codegraph_dir = get_codegraph_dir(project_root);
    let db_path = codegraph_dir.join("codegraph.db");
    if db_path.exists() {
        bail!("CodeGraph already initialized in {}", project_root.display());
    }

    fs::create_dir_all(&codegraph_dir)
        .with_context(|| format!("failed to create {}", codegraph_dir.display()))?;

    let gitignore_path = codegraph_dir.join(".gitignore");
    if !gitignore_path.exists() {
        fs::write(&gitignore_path, GITIGNORE_CONTENT)
            .with_context(|| format!("failed to write {}", gitignore_path.display()))?;
    }

    Ok(())
}

pub fn remove_directory(project_root: &Path) -> Result<()> {
    let codegraph_dir = get_codegraph_dir(project_root);
    if !codegraph_dir.exists() {
        return Ok(());
    }

    let metadata = fs::symlink_metadata(&codegraph_dir)
        .with_context(|| format!("failed to inspect {}", codegraph_dir.display()))?;

    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(&codegraph_dir)
            .with_context(|| format!("failed to remove {}", codegraph_dir.display()))?;
        return Ok(());
    }

    fs::remove_dir_all(&codegraph_dir)
        .with_context(|| format!("failed to remove {}", codegraph_dir.display()))?;
    Ok(())
}

pub fn validate_directory(project_root: &Path) -> Result<Vec<String>> {
    let mut errors = Vec::new();
    let codegraph_dir = get_codegraph_dir(project_root);

    if !codegraph_dir.exists() {
        errors.push("CodeGraph directory does not exist".to_string());
        return Ok(errors);
    }

    if !codegraph_dir.is_dir() {
        errors.push(".codegraph exists but is not a directory".to_string());
        return Ok(errors);
    }

    let gitignore_path = codegraph_dir.join(".gitignore");
    if !gitignore_path.exists() {
        fs::write(&gitignore_path, GITIGNORE_CONTENT)
            .with_context(|| format!("failed to repair {}", gitignore_path.display()))?;
    }

    Ok(errors)
}
