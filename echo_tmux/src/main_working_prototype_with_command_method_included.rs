use std::io::{self, Write};
use std::process::{Command}; // Only Command needed in blocking parts — remove unused imports
use std::path::PathBuf;
use std::collections::HashMap;
// use std::collections::HashMap; # Removed — no longer needed directly
use tokio::{
    net::UnixStream,
    io::{AsyncReadExt, AsyncWriteExt},
    signal::unix::{signal, SignalKind},
};
use dirs_next as dirs; // Modern replacement for old 'dirs' crate
use serde_json::{self, Value, json};
use reqwest::blocking::Client; // If you want blocking API calls — otherwise remove this and make everything async
// use futures_util::future::FutureExt; # Removed — no longer needed
use once_cell::sync::Lazy;
use tokio::sync::Mutex; // Import the right Mutex for async code
use anyhow::{bail, Result}; // Importing `anyhow` macro explicitly

// ANSI color codes (for non-tty output)
const LIGHT_BLUE: &str = "\x1b[94m";
const YELLOW: &str = "\x1b[33m";
const RESET_COLOR: &str = "\x1b[0m";

// Constants
const MODEL_NAME: &str = "Echo"; // Change if different
const API_URL: &str = "http://localhost:8080/v1/chat/completions"; // Your local Qwen-7B server port

const SYSTEM_PROMPT: &str = r#"
You are Echo, Eric's local red team AI sidekick.

You ONLY use the session format for ALL actions. Never use COMMAND: anymore.

Rules:
- Every time you want to run a command or tool, you MUST use: session:NAME command here
- If the session does not exist yet, the wrapper will automatically create it.
- After a session is created, keep using the same session:NAME for follow-up commands.
- Only do ONE action per response. Wait for the result before deciding the next step.
- Be direct, efficient, and tool-first. No unnecessary politeness or explanations.

Examples:
User: Check open ports on the local network
Echo: session:recon nmap -sV 192.168.1.0/24

User: Start Metasploit
Echo: session:msf msfconsole -q

User: Show current users
Echo: session:shell whoami && id

Stay sharp and always use session:NAME format.
"#;


// Thread-safe static for active sessions (pipe paths)
pub static ACTIVE_SESSIONS: Lazy<Mutex<HashMap<String, (String, String)>>> = Lazy::new(|| {
    Mutex::new(HashMap::new())
});

static SHUTDOWN_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[tokio::main]
async fn main() -> Result<()> { // Change to return Result for proper error handling
    println!("Echo Rust Wrapper v2 – Async Tool Calls with Named Pipes");
    println!("Type 'quit' or 'exit' to stop.\n");

    // Handle graceful shutdowns (SIGINT/SIGTERM)
    let mut termination = signal(SignalKind::terminate()).expect("Failed to set up SIGTERM handler");
    let mut interrupt = signal(SignalKind::interrupt()).expect("Failed to set up SIGINT handler");

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = termination.recv() => { SHUTDOWN_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst); break; },
                _ = interrupt.recv() => { SHUTDOWN_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst); break; }
            }
        }
    });

    let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/eric")); // Update fallback

    // Load initial context if exists
    let mut context_content = String::new();
    let context_path = home_dir.join("Documents").join("Echo_context.txt");

    if tokio::fs::metadata(&context_path).await.is_ok() {
        context_content = tokio::fs::read_to_string(&context_path)
            .await
            .expect("Failed to read context file");
    }

    // Ensure ~/Documents exists
    tokio::fs::create_dir_all(home_dir.join("Documents"))
        .await
        .expect("Failed to create Documents dir");

    let full_system_prompt = format!("{}\n\n{}", SYSTEM_PROMPT.trim(), context_content.trim());

    // Log session start
    save_chat_log_entry(&home_dir, "", &full_system_prompt, "SESSION_START").await?;

    let mut messages = vec![
        json!({"role": "system", "content": full_system_prompt}),
    ];

    println!("Echo: Ready. Type 'quit' or 'exit' to end session.\n");

    loop {
        print!("You: ");
        io::stdout().flush()?;
        let mut user_input = String::new();
        io::stdin()
            .read_line(&mut user_input)
            .expect("Failed to read line");
        let trimmed_input = user_input.trim();

        // === EXIT CHECK ===
        if trimmed_input.eq_ignore_ascii_case("quit") || trimmed_input.eq_ignore_ascii_case("exit") {
            println!("Session ended.");

            save_chat_log_entry(&home_dir, "", "", "SESSION_END").await.unwrap();

            break;
        }

        if SHUTDOWN_REQUESTED.load(std::sync::atomic::Ordering::SeqCst) {
            println!("\nGraceful shutdown initiated...");
            clean_up_sessions().await?;
            println!("All sessions terminated. Goodbye!");
            return Ok(()); // Exit cleanly
        }

        // Log user message
        save_chat_log_entry(&home_dir, trimmed_input, "", "user").await.unwrap();

        messages.push(json!({
            "role": "user",
            "content": trimmed_input,
        }));

        println!("Echo: Sending request to local model...\n");

        // Prepare and send API request to Qwen-7B server (your local model)
        let payload = json!({
            "model": MODEL_NAME,
            "messages": &messages,
            "temperature": 0.3,
            "max_tokens": 1024
        });

        println!("Echo: Sending request to local model...\n");

        // Send POST request to your Qwen-7B server (port 8080 by default)
       // Send async POST request to the model
        let response_text = match reqwest::Client::new()
            .post(API_URL)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
        {
            Ok(res) => {
                if res.status().is_success() {
                    let body_str = res.text().await.unwrap_or_default();
                    match serde_json::from_str::<Value>(&body_str) {
                        Ok(parsed) => parsed["choices"][0]["message"]["content"]
                            .as_str()
                            .unwrap_or("")
                            .trim()
                            .to_string(),
                        Err(_) => "Invalid JSON from API response.".to_string(),
                    }
                } else {
                    format!("API request failed with status: {}", res.status())
                }
            }
            Err(e) => format!(
                "Request to {} failed: {}. Is your local model server running?",
                API_URL, e
            ),
        };

        // Log assistant response
        save_chat_log_entry(&home_dir, "", &response_text, "assistant").await.unwrap();

        let mut handled = false;

        // Check for session creation commands (e.g., session:shell bash -i)
        if let Some((session_name, command)) = extract_session_command(&response_text) {
            println!("{}Echo: Creating/reusing session '{}' and running '{}'.{}", LIGHT_BLUE, &session_name, &command, RESET_COLOR);

            // Start or reuse the session
            start_or_reuse_session(home_dir.clone(), &session_name, &command).await?;

            let output = execute_in_session(home_dir.clone(), &session_name, command.to_string()).await?; // Add .await?

            for line in output.lines() { // Works on String from above fn
                if line.contains("ERROR") || line.contains("failed") {
                    println!("{}{}\n{}", YELLOW, line, RESET_COLOR);
                } else {
                    println!("{}{}\n{}", LIGHT_BLUE, line.trim(), RESET_COLOR);
                }

                // Log command execution result
                save_chat_log_entry(&home_dir, "", &line, &session_name).await.unwrap();
            }

            handled = true;

        // Check for running commands in existing sessions (e.g., tool_name: run lsblk)
        } else if let Some((session_name, sub_command)) = extract_run_command(&response_text) {
            match execute_in_session(home_dir.clone(), &session_name, format!("{} {}", "run", sub_command.trim())).await? { // Add .await?
                output => {
                    println!("{}Echo: Output from session '{}':{}", LIGHT_BLUE, &session_name, RESET_COLOR);

                    for line in output.lines() { // Works on String from above fn
                        if line.contains("ERROR") || line.contains("failed") {
                            println!("{}{}\n{}", YELLOW, line, RESET_COLOR);
                        } else {
                            println!("{}{}\n{}", LIGHT_BLUE, line.trim(), RESET_COLOR);
                        }
                        save_chat_log_entry(&home_dir, "", &line, &session_name).await.unwrap();
                    }
                },
            }

            handled = true;

        // Check for ending sessions (e.g., end_session: shell)
        } else if let Some(session_name) = extract_end_command(&response_text) {
            match end_session(home_dir.clone(), &session_name).await {
                Ok(_) => {
                    println!("{}Echo: Session '{}' ended.", LIGHT_BLUE, &session_name);

                    // Log session termination
                    save_chat_log_entry(&home_dir, "", "Session terminated", &session_name).await.unwrap();
                },
                Err(e) => {
                    println!("Echo: Failed to end session '{}': {}", &session_name, e);
                }
            }

            handled = true;
        } else if let Some(command) = extract_command(&response_text) { // Existing COMMAND: handling
            println!("{}Echo: Executing command:{}\n{}\n{}", LIGHT_BLUE, RESET_COLOR, command.trim(), RESET_COLOR);

            // Run locally (not in session)
            let output = Command::new("sh")
                .arg("-c")
                .arg(command.trim())
                .output()
                .expect("Failed to execute command");

            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();

            // Display output
            if !stdout.is_empty() {
                println!("{}Echo:\n{}\n{}", LIGHT_BLUE, &stdout.trim(), RESET_COLOR);
            }

            if !stderr.is_empty() {
                println!("{}Errors/Warnings:\n{}\n---", YELLOW, &stderr.trim());
            }

            // Save full response
            let full_response = format!("[COMMAND_OUTPUT]\nSTDOUT:\n{}\nSTDERR:\n{}", stdout, stderr);
            save_chat_log_entry(&home_dir, "", &full_response, "main").await.unwrap();

            handled = true;
        } else {
            // Plain text response (no tool call)
            println!("{}Echo:\n{}\n{}", LIGHT_BLUE, response_text.trim(), RESET_COLOR);

            if !response_text.is_empty() && trimmed_input != "quit" && trimmed_input != "exit" {
                messages.push(json!({
                    "role": "assistant",
                    "content": &response_text,
                }));
            }
        }

        // If nothing was handled and it's not an exit, we're done this turn
        if !handled && response_text.trim() != "" {
            println!("{}Echo: No further actions required.", LIGHT_BLUE);
        }

    } // end loop

    clean_up_sessions().await?;
    println!("\nSession ended normally. Goodbye!");

    Ok(())
}

// Start or reuse a named session (creates named pipes)
async fn start_or_reuse_session(home: PathBuf, name: &str, command: &str) -> Result<()> {
    let pipe_root = home.join("Documents").join(".echo_pipes");
    tokio::fs::create_dir_all(&pipe_root).await?;

    let stdin_path = pipe_root.join(format!("{}.stdin", name));
    let stdout_path = pipe_root.join(format!("{}.stdout", name));

    // Check if session already exists
    {
        let sessions = ACTIVE_SESSIONS.lock().await;
        if sessions.contains_key(name) {
            println!("Echo: Reusing existing session '{}'.", name);
            return Ok(());
        }
    }

    println!("Echo: Creating new session '{}' with command: {}", name, command);

    // Create named pipes (blocking)
    let status = Command::new("mkfifo")
        .args([&stdin_path, &stdout_path])
        .status() // Blocking — correct for creating pipes
        .expect("Failed to create named pipe");

    if !status.success() {
        bail!("Failed to create named pipes for session '{}': {:?}", name, status);
    }

    // Start shell in background (blocking):
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .args([
            format!("exec {command} < {} > {}", stdin_path.display(), stdout_path.display())
        ])
        .spawn()?;

    // Wait for the command to finish and check if it was successful
    let status = child.wait().unwrap_or_default();
    if !status.success() {
    bail!("Failed to start session '{}': {:?}", name, status);
    }

    // Insert into sessions:
    let mut sessions = ACTIVE_SESSIONS.lock().await;
    sessions.insert(name.to_string(), (stdin_path.to_string_lossy().to_string(), stdout_path.to_string_lossy().to_string()));

    Ok(())
}

// Execute a command inside an existing session
async fn execute_in_session(_home: PathBuf, session_name: &str, command: String) -> Result<String> {
    let mut buffer = Vec::<u8>::new();

    // Lock sessions — avoid lazy_static entirely (recommended):
    let sessions = ACTIVE_SESSIONS.lock().await;

    if let Some((stdin_path, stdout_path)) = sessions.get(session_name) {
        // Write to stdin pipe:
        match UnixStream::connect(stdin_path).await {
            Ok(mut stdin_pipe) => {
                stdin_pipe.write_all(command.as_bytes()).await?;
                stdin_pipe.shutdown().await?; // Close after sending
            },
            Err(_) => bail!("Session not found: {}", session_name),
        }

        // Read from stdout pipe:
        match UnixStream::connect(stdout_path).await {
            Ok(mut stdout_pipe) => {
                while let Ok(size) = stdout_pipe.read_buf(&mut buffer).await {
                    if size == 0 { break; } // EOF
                }
                return Ok(String::from_utf8_lossy(&buffer).to_string());
            },
            Err(_) => bail!("Session not found: {}", session_name),
        }
    } else {
        bail!("Session '{}' not active.", session_name);
    }

}

// Kill a named session
async fn end_session(_home_dir: PathBuf, name: &str) -> Result<()> { // Removed unused log_dir
    let mut sessions = ACTIVE_SESSIONS.lock().await;
    if let Some((stdin_path, stdout_path)) = sessions.remove(name) {
        println!("Echo: Terminating and cleaning up session '{}'.", name);

        // Close pipes (kills the process)
        tokio::fs::remove_file(&stdin_path).await?;
        tokio::fs::remove_file(&stdout_path).await?;

        Ok(())
    } else {
        bail!("Session '{}' not active.", name);
    }
}

// Save chat log entries to one JSONL file without timestamps or extra metadata
async fn save_chat_log_entry(log_dir: &PathBuf, user_message: &str, assistant_response: &str, from: &str) -> Result<()> {
    let mut messages_json = Vec::new();

    if !user_message.is_empty() {
        messages_json.push(json!({
            "from": "human",
            "value": user_message.trim()
        }));
    }

    if !assistant_response.is_empty() || from.contains("SESSION_START") || from.contains("SESSION_END") {
        let assistant_value = if from.contains("SESSION_START") || from.contains("SESSION_END") {
            format!("Session event: {}", from)
        } else {
            assistant_response.trim().to_string()
        };

        messages_json.push(json!({
            "from": "gpt",
            "value": assistant_value
        }));
    }

    if !from.is_empty() && from != "main" { // Log sessions separately for clarity
        messages_json.push(json!({
            "from": "system",
            "value": format!("Session: {}", from)
        }));
    }

    let log_entry = serde_json::to_string(&messages_json)?;

    tokio::fs::create_dir_all(log_dir).await?;
    let file_path = log_dir.join("echo_chat.jsonl");

    match std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&file_path) {

        Ok(mut file) => {
            // Explicitly handle writeln! error
            if let Err(e) = writeln!(file, "{}", log_entry) {
                bail!("Failed to write chat log: {}", e);
            }
        },

        Err(e) => bail!("Error opening {}: {}", file_path.display(), e),
    };

    Ok(())
}


// Kill all sessions cleanly
async fn clean_up_sessions() -> Result<()> {
    // Lock once and iterate — no early drop needed
    let sessions = ACTIVE_SESSIONS.lock().await;

    // Collect tasks: map iter to futures instead of push manually
    let cleanup_tasks = sessions.iter().map(|(name, paths)| {
        let stdin_path = PathBuf::from(paths.0.clone());
        let stdout_path = PathBuf::from(paths.1.clone());

        cleanup_session(
            PathBuf::from("/home/eric/Documents"),
            name.as_str(), // Directly pass &str
            stdin_path,
            stdout_path
        )
    }).collect::<Vec<_>>();

    // Join all tasks without dropping sessions early
    futures_util::future::join_all(cleanup_tasks).await;

    Ok(())
}

async fn cleanup_session(_home_dir: PathBuf, name: &str, stdin_path: PathBuf, stdout_path: PathBuf) -> Result<()> {
    let mut sessions = ACTIVE_SESSIONS.lock().await;

    if let Some((_, _)) = sessions.remove(name) {
        println!("Echo: Terminating and cleaning up session '{}'.", name);

        // Close pipes (kills the process)
        tokio::fs::remove_file(stdin_path).await?;
        tokio::fs::remove_file(stdout_path).await?;

        Ok(())
    } else {
        bail!("Session '{}' not active.", name);
    }
}


// Extract session creation command (e.g., session:shell bash -i)
fn extract_session_command(response_text: &str) -> Option<(String, String)> {
    for line in response_text.lines() {
        if let Some((session_part, rest)) = line.trim().split_once("session:") {
            if let Some(command) = rest.trim().strip_prefix(' ') {
                return Some((
                    session_part.trim().to_string(),
                    command.to_string()
                ));
            }
        }
    }

    None
}

// Extract sub-command to run in an existing session (e.g., tool_name: run lsblk)
fn extract_run_command(response_text: &str) -> Option<(String, String)> {
    for line in response_text.lines() {
        if let Some((session_part, command)) = line.trim().split_once("tool_name: run") {
            if let Some(session_name) = session_part.split_whitespace().next() {
                return Some((
                    session_name.to_string(),
                    format!("{} {}", "run", command.trim())
                ));
            }
        }
    }

    None
}

// Extract end-session command (e.g., end_session: shell)
fn extract_end_command(response_text: &str) -> Option<String> {
    for line in response_text.lines() {
        if let Some(session_name) = line.trim().strip_prefix("end_session:") {
            return Some(session_name.to_string());
        }
    }

    None
}

// Extract existing COMMAND: lines (for local execution)
fn extract_command(response_text: &str) -> Option<String> {
    for line in response_text.lines() {
        if let Some(command) = line.trim().strip_prefix("COMMAND:") {
            return Some(command.to_string());
        }
    }

    None
}

