use anyhow::{Context, Result, bail};
use std::env;
use std::fs;
use std::process::Command;

pub fn edit(content: &str, filename: &str) -> Result<String> {
    let editor = env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
    let temp_dir = env::temp_dir();
    let safe_filename = filename.replace('/', "_");
    let temp_file = temp_dir.join(format!("tome_{safe_filename}"));

    fs::write(&temp_file, content).context("Failed to write temp file")?;

    let status = Command::new(&editor)
        .arg(&temp_file)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .with_context(|| format!("Failed to launch editor: {editor}"))?;

    if !status.success() {
        bail!("Editor exited with status {}", status);
    }

    let edited = fs::read_to_string(&temp_file).context("Failed to read edited file")?;
    fs::remove_file(&temp_file).ok();
    Ok(edited.trim().to_string())
}
