//! Locations of the HUD's own state and of the read-only agent logs.

use std::env;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct HudPaths {
    /// Installed plugin directory (contains `bin`, `locales`, `.codex-plugin`).
    pub plugin_root: PathBuf,
    /// `%LOCALAPPDATA%\CodexMonitorHUD` — settings, registry, signals, user themes.
    pub state_root: PathBuf,
    pub config_path: PathBuf,
    pub home: PathBuf,
    /// `~/.codex`
    pub codex_home: PathBuf,
    /// `~/.codex-deepseek`
    pub deepseek_home: PathBuf,
    /// `CLAUDE_CONFIG_DIR` or `~/.claude`
    pub claude_root: PathBuf,
    /// `DSH_HOME` or `~/.dsh` (DeepSeek Harness)
    pub dsh_root: PathBuf,
}

impl HudPaths {
    pub fn resolve() -> Self {
        let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
        let exe_dir = exe.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        // Installed layout: <plugin>/bin/AgentMonitorHUD.exe. During development the
        // executable lives in the Cargo target directory; fall back to the source tree.
        let plugin_root = env::var_os("CODEX_MONITOR_HUD_PLUGIN_ROOT")
            .map(PathBuf::from)
            .or_else(|| {
                exe_dir
                    .ancestors()
                    .take(4)
                    .find(|dir| dir.join(".codex-plugin").join("plugin.json").exists())
                    .map(Path::to_path_buf)
            })
            .unwrap_or_else(|| exe_dir.clone());
        let home = env::var_os("CODEX_MONITOR_HUD_TEST_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("USERPROFILE").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));
        let state_root = if let Some(root) = env::var_os("CODEX_MONITOR_HUD_STATE_ROOT") {
            PathBuf::from(root)
        } else {
            let local = env::var_os("CODEX_MONITOR_HUD_TEST_LOCALAPPDATA")
                .or_else(|| env::var_os("LOCALAPPDATA"))
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("AppData").join("Local"));
            local.join("CodexMonitorHUD")
        };
        let claude_root = env::var_os("CODEX_MONITOR_HUD_CLAUDE_ROOT")
            .or_else(|| env::var_os("CLAUDE_CONFIG_DIR"))
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".claude"));
        let dsh_root = env::var_os("CODEX_MONITOR_HUD_DSH_ROOT")
            .or_else(|| env::var_os("DSH_HOME"))
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".dsh"));
        HudPaths {
            config_path: state_root.join("settings.json"),
            codex_home: home.join(".codex"),
            deepseek_home: home.join(".codex-deepseek"),
            plugin_root,
            state_root,
            home,
            claude_root,
            dsh_root,
        }
    }

    pub fn state(&self, name: &str) -> PathBuf {
        self.state_root.join(name)
    }

    pub fn hosts_dir(&self) -> PathBuf {
        self.state_root.join("hosts")
    }

    pub fn notifications_dir(&self) -> PathBuf {
        self.state_root.join("notifications")
    }

    pub fn claude_state_dir(&self) -> PathBuf {
        self.state_root.join("claude")
    }

    pub fn registry_path(&self) -> PathBuf {
        self.state_root.join("task-registry.json")
    }

    pub fn user_themes_dir(&self) -> PathBuf {
        self.state_root.join("themes")
    }

    pub fn log_path(&self) -> PathBuf {
        self.state_root.join("hud.log")
    }

    pub fn exe_path() -> PathBuf {
        env::current_exe().unwrap_or_else(|_| PathBuf::from("AgentMonitorHUD.exe"))
    }
}
