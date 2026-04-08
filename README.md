# Echo TMUX

A lightweight Rust wrapper that turns a local LLM (currently "Echo") into a capable red team / pentesting agent or any CLI workflow with persistent tmux sessions.

### Features
- Persistent named sessions via tmux (`SESSION:NAME command`)
- One-off shell commands (`COMMAND: command`)
- Clean ShareGPT-style JSONL logging (one full turn per line)
- Context file support
- Basic safety filtering for dangerous commands
- Clean chat output
- Currently building datasets and testing
- Build details in [progress_log.md](https://github.com/charlesericwilson-portfolio/Echo_tmux/blob/main/echo_tmux/Doc/progress_log.md)
- Screenshots [Basic command execution and output](https://github.com/charlesericwilson-portfolio/Echo_tmux/tree/main/echo_tmux/screenshots)

### Quick Start

1. **Install dependencies**
   ```bash
   sudo apt install tmux
   sudo apt install cargo
   sudo apt install rustup
2. **Make sure your llama.cpp server is running on port 8080**

3. **Build and run the Rust version**

  ```bash
  cd [build directory]
  cargo build --release
  ./target/release/echo_rust_wrapper
  ```
Builds in progress [Echo Agent Proxy](https://github.com/charlesericwilson-portfolio/Echo_agent_proxy)
