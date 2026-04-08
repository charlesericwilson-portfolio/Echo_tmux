pub fn extract_session_command(response_text: &str) -> Option<(String, String)> {
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

pub fn extract_run_command(response_text: &str) -> Option<(String, String)> {
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

pub fn extract_end_command(response_text: &str) -> Option<String> {
    for line in response_text.lines() {
        if let Some(session_name) = line.trim().strip_prefix("end_session:") {
            return Some(session_name.to_string());
        }
    }

    None
}

pub fn extract_command(response_text: &str) -> Option<String> {
    for line in response_text.lines() {
        if let Some(command) = line.trim().strip_prefix("COMMAND:") {
            return Some(command.to_string());
        }
    }

    None
}
