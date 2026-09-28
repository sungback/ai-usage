//! macOS LaunchAgent 기반 로그인 시 자동 실행 관리.

use std::path::PathBuf;

const LAUNCH_AGENT_LABEL: &str = "com.sungback.aiusage";
const LEGACY_LAUNCH_AGENT_LABELS: [&str; 2] =
    ["com.sungback.myaimonitor", "com.codezeno.claudecodeusagemonitor"];

fn launch_agent_plist_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| {
        home.join("Library")
            .join("LaunchAgents")
            .join(format!("{LAUNCH_AGENT_LABEL}.plist"))
    })
}

fn legacy_launch_agent_plist_paths() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    LEGACY_LAUNCH_AGENT_LABELS
        .iter()
        .map(|label| {
            home.join("Library")
                .join("LaunchAgents")
                .join(format!("{label}.plist"))
        })
        .collect()
}

fn remove_legacy_launch_agents() {
    for path in legacy_launch_agent_plist_paths() {
        let _ = std::fs::remove_file(path);
    }
}

pub fn is_startup_enabled() -> bool {
    launch_agent_plist_path()
        .map(|path| path.exists())
        .unwrap_or(false)
        || legacy_launch_agent_plist_paths()
            .iter()
            .any(|path| path.exists())
}

pub fn set_startup_enabled(enable: bool) {
    let Some(plist_path) = launch_agent_plist_path() else {
        return;
    };

    if enable {
        let exe_path = match std::env::current_exe() {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(_) => return,
        };

        if let Some(parent) = plist_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let plist_content = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
                 <key>Label</key>\n\
                 <string>{LAUNCH_AGENT_LABEL}</string>\n\
                 <key>ProgramArguments</key>\n\
                 <array>\n\
                     <string>{exe_path}</string>\n\
                 </array>\n\
                 <key>RunAtLoad</key>\n\
                 <true/>\n\
                 <key>ProcessType</key>\n\
                 <string>Interactive</string>\n\
             </dict>\n\
             </plist>\n"
        );

        let _ = std::fs::write(&plist_path, plist_content);
        remove_legacy_launch_agents();
    } else {
        let _ = std::fs::remove_file(&plist_path);
        remove_legacy_launch_agents();
    }
}
