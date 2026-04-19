// log.rs
use std::path::PathBuf;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use anyhow::Result;
use serde_json::json;

pub async fn save_chat_log_entry(
    log_dir: &PathBuf,
    user_message: &str,
    assistant_response: &str,
) -> Result<()> {
    tokio::fs::create_dir_all(log_dir).await?;

    let file_path = log_dir.join("echo_chat.jsonl");

    let mut messages = Vec::new();

    if !user_message.trim().is_empty() {
        messages.push(json!({
            "role": "user",
            "content": user_message.trim()
        }));
    }

    if !assistant_response.trim().is_empty() {
        messages.push(json!({
            "role": "assistant",
            "content": assistant_response.trim()
        }));
    }

    if messages.is_empty() {
        return Ok(());
    }

    let log_entry = json!({
        "messages": messages
    });

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)
        .await?;

    let line = format!("{}\n", log_entry.to_string());
    file.write_all(line.as_bytes()).await?;
    file.flush().await?;

    Ok(())
}
