use tokio::{
    net::UnixStream,
    io::{AsyncReadExt, AsyncWriteExt},
};
use anyhow::{bail, Result};

pub use crate::ACTIVE_SESSIONS;
// SHUTDOWN_REQUESTED is not needed in sessions.rs → we remove it from the re-export

// Start or reuse a named session (creates named pipes)
pub async fn start_or_reuse_session(home: std::path::PathBuf, name: &str, command: &str) -> Result<()> {
    let pipe_root = home.join("Documents").join(".echo_pipes");
    tokio::fs::create_dir_all(&pipe_root).await?;

    // Create named pipes (blocking)
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

    std::process::Command::new("mkfifo")
        .args([&stdin_path, &stdout_path])
        .status() // Blocking — correct for creating pipes
        .expect("Failed to create named pipe");

    // Start shell in background (blocking):
    let mut child = std::process::Command::new("/bin/sh")
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
pub async fn execute_in_session(_home: std::path::PathBuf, session_name: &str, command: String) -> Result<String> {
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
pub async fn end_session(_home_dir: std::path::PathBuf, name: &str) -> Result<()> { // Removed unused log_dir
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

// Kill all sessions cleanly
pub async fn clean_up_sessions() -> Result<()> {
    // Lock once and iterate — no early drop needed
    let sessions = ACTIVE_SESSIONS.lock().await;

    // Collect tasks: map iter to futures instead of push manually
    let cleanup_tasks = sessions.iter().map(|(name, paths)| {
        let stdin_path = std::path::PathBuf::from(paths.0.clone());
        let stdout_path = std::path::PathBuf::from(paths.1.clone());

        cleanup_session(
            std::path::PathBuf::from("/home/eric/Documents"),
            name.as_str(), // Directly pass &str
            stdin_path,
            stdout_path
        )
    }).collect::<Vec<_>>();

    // Join all tasks without dropping sessions early
    futures_util::future::join_all(cleanup_tasks).await;

    Ok(())
}

async fn cleanup_session(_home_dir: std::path::PathBuf, name: &str, stdin_path: std::path::PathBuf, stdout_path: std::path::PathBuf) -> Result<()> {
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
