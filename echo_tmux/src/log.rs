use std::path::PathBuf;
use serde_json::json;
use anyhow::{bail, Result};
use std::io::Write;

//pub use crate::{ACTIVE_SESSIONS, SHUTDOWN_REQUESTED, LIGHT_BLUE, YELLOW, RESET_COLOR};

// Save chat log entries to one JSONL file without timestamps or extra metadata
// log.rs
use std::fs::OpenOptions;

pub async fn save_chat_log_entry(
    log_dir: &PathBuf,
    user_message: &str,
    assistant_response: &str,
    from: &str,
) -> Result<()> {
    let mut entries = Vec::new();

    if !user_message.is_empty() {
        entries.push(json!({
            "from": "human",
            "value": user_message.trim()
        }));
    }

    if !assistant_response.is_empty() || from.contains("SESSION_START") || from.contains("SESSION_END") {
        let value = if from.contains("SESSION_START") || from.contains("SESSION_END") {
            format!("Session event: {}", from)
        } else {
            assistant_response.trim().to_string()
        };

        entries.push(json!({
            "from": "gpt",
            "value": value
        }));
    }

    if !from.is_empty() && from != "main" && from != "assistant" && from != "user" {
        entries.push(json!({
            "from": "system",
            "value": format!("Session: {}", from)
        }));
    }

    let log_entry = serde_json::to_string(&entries)?;

    tokio::fs::create_dir_all(log_dir).await?;

    let file_path = log_dir.join("echo_chat.jsonl");

    let mut file = match OpenOptions::new()
        .append(true)
        .create(true)
        .open(&file_path)
    {
        Ok(file) => file,
        Err(e) => bail!("Error opening {}: {}", file_path.display(), e),
    };

    writeln!(file, "{}", log_entry)
        .map_err(|e| anyhow::anyhow!("Failed to write to log: {}", e))?;

    Ok(())
}
