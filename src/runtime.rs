//! The engine thread: owns the task table, reacts to watcher wakes and UI
//! commands, adapts its cadence, writes the registry and publishes views.

use crate::config::Settings;
use crate::engine::{self, Engine, View, allowance, notices, registry};
use crate::hook;
use crate::locale::Catalog;
use crate::paths::HudPaths;
use crate::util::time;
use crate::watch;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub enum Command {
    Paths(Vec<PathBuf>),
    Rescan,
    Settings(Box<Settings>),
    Dismiss(String),
    DismissNotice(String),
    HoldExits(bool),
    Pause(bool),
    Official(Option<allowance::Official>),
    /// The stored DeepSeek key changed: re-read the balance now.
    DshKeyChanged,
    DshBalance(Result<engine::dsh_balance::Balance, engine::dsh_balance::Failure>),
    Quit,
}

pub struct Shared {
    view: Mutex<Arc<View>>,
}

impl Shared {
    pub fn view(&self) -> Arc<View> {
        self.view.lock().unwrap().clone()
    }
}

pub struct Handle {
    pub tx: Sender<Command>,
    pub shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
}

impl Handle {
    pub fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Command::Quit);
    }
}

pub fn spawn(paths: HudPaths, settings: Settings, catalog: Catalog, notify: impl Fn() + Send + 'static) -> Handle {
    let (tx, rx) = channel::<Command>();
    let shared = Arc::new(Shared { view: Mutex::new(Arc::new(View::default())) });
    let stop = Arc::new(AtomicBool::new(false));
    let engine = Engine::new(paths.clone(), settings);
    let mut roots = engine.watch_roots();
    roots.push((paths.state_root.clone(), true));
    let watch_tx = tx.clone();
    watch::spawn(roots, stop.clone(), move |event| {
        let command = match event {
            watch::WatchEvent::Paths(paths) => {
                // Session logs, title indexes, DSH checkpoints, hook events and
                // notices are all JSON/JSONL; other churn (SQLite WAL, temp
                // files) must not wake the engine.
                let paths: Vec<PathBuf> = paths
                    .into_iter()
                    .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("jsonl") || e.eq_ignore_ascii_case("json")))
                    .collect();
                if paths.is_empty() {
                    return;
                }
                Command::Paths(paths)
            }
            watch::WatchEvent::Overflow => Command::Rescan,
        };
        let _ = watch_tx.send(command);
    });
    let thread_shared = shared.clone();
    let self_tx = tx.clone();
    std::thread::Builder::new()
        .name("hud-engine".into())
        .stack_size(1024 * 1024)
        .spawn(move || run(engine, catalog, rx, self_tx, thread_shared, notify))
        .expect("engine thread");
    Handle { tx, shared, stop }
}

fn run(mut engine: Engine, catalog: Catalog, rx: Receiver<Command>, self_tx: Sender<Command>, shared: Arc<Shared>, notify: impl Fn()) {
    let paths = engine.paths.clone();
    let notifications_dir = paths.notifications_dir();
    let events_dir = paths.claude_state_dir().join("events");
    let _ = std::fs::create_dir_all(&notifications_dir);
    let _ = std::fs::create_dir_all(&events_dir);
    let registry_path = paths.registry_path();
    let mut registry_cache = String::new();
    let mut quota = registry::QuotaGuard::default();
    let mut last_view: Option<View> = None;
    let mut last_notice_scan = time::MIN;
    let mut responsive_until = 0i64;
    let mut official_running = false;
    let mut official_next = time::now_ms() + 10_000;
    let mut balance_running = false;
    let mut balance_next = time::now_ms() + 3_000;

    let now = time::now_ms();
    engine.apply_hook_events(hook::drain_events(&paths), now);
    engine.full_scan(now);
    engine.poll_backlog(now);

    loop {
        let now = time::now_ms();
        let cadence = if now < responsive_until || engine.has_backlog() {
            drop(crate::util::prof::probe(if engine.has_backlog() { crate::util::prof::FAST_BACKLOG } else { crate::util::prof::FAST_RESPONSIVE }));
            250
        } else if engine.busiest_status(now) != "idle" {
            800
        } else {
            1500
        };
        let mut commands = Vec::new();
        match rx.recv_timeout(Duration::from_millis(cadence)) {
            Ok(cmd) => commands.push(cmd),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                engine.ledger.save(time::now_ms());
                return;
            }
        }
        while let Ok(cmd) = rx.try_recv() {
            commands.push(cmd);
        }
        let now = time::now_ms();
        let _loop_probe = crate::util::prof::probe(crate::util::prof::LOOP);
        crate::util::prof::maybe_report(now);
        let mut changed_paths: Vec<PathBuf> = Vec::new();
        let mut rescan = false;
        let mut check_notices = now - last_notice_scan >= 30_000;
        let mut check_hooks = false;
        for cmd in commands {
            match cmd {
                Command::Quit => {
                    engine.ledger.save(now);
                    return;
                }
                Command::Paths(paths_changed) => {
                    for path in paths_changed {
                        if path.starts_with(&notifications_dir) {
                            check_notices = true;
                        } else if path.starts_with(&events_dir) {
                            check_hooks = true;
                        } else if !path.starts_with(&paths.state_root)
                            && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("jsonl") || e.eq_ignore_ascii_case("json"))
                        {
                            // Only session logs, title indexes and DSH checkpoints matter;
                            // database/WAL churn must not speed the loop up.
                            changed_paths.push(path);
                        }
                    }
                }
                Command::Rescan => rescan = true,
                Command::Settings(settings) => {
                    engine.update_settings(*settings);
                    rescan = true;
                }
                Command::Dismiss(key) => engine.dismiss(&key),
                Command::DismissNotice(key) => engine.dismiss_notice(&key),
                Command::HoldExits(hold) => engine.hold_terminal_exits = hold,
                Command::Pause(paused) => {
                    engine.paused = paused;
                    engine.revision += 1;
                }
                Command::DshKeyChanged => {
                    balance_next = now;
                    if !crate::secrets::has_deepseek_key(&paths) {
                        engine.dsh_balance = None;
                        engine.dsh_balance_failure = None;
                        engine.revision += 1;
                    }
                }
                Command::DshBalance(result) => {
                    balance_running = false;
                    match result {
                        Ok(balance) => {
                            engine.dsh_balance = Some((balance, now));
                            engine.dsh_balance_failure = None;
                        }
                        Err(failure) => engine.dsh_balance_failure = Some(failure),
                    }
                    engine.revision += 1;
                }
                Command::Official(result) => {
                    official_running = false;
                    if engine.official_allowance != result {
                        engine.official_allowance = result;
                        engine.revision += 1;
                    }
                }
            }
        }
        if check_hooks || !engine.initial_scan_complete {
            let events = hook::drain_events(&paths);
            if !events.is_empty() {
                engine.apply_hook_events(events, now);
                responsive_until = now + 2000;
            }
        }
        if !engine.paused {
            if rescan || engine.full_scan_due(now) {
                let _p = crate::util::prof::probe(crate::util::prof::FULL_SCAN);
                engine.full_scan(now);
            } else if engine.activity_query_due(now) {
                let _p = crate::util::prof::probe(crate::util::prof::ACTIVITY);
                engine.full_scan_activity(now);
            }
            if !changed_paths.is_empty() {
                let _p = crate::util::prof::probe(crate::util::prof::PATHS);
                engine.on_paths(&changed_paths, now);
                responsive_until = now + 2000;
            }
            engine.refresh_claude_quota(now);
            let _p = crate::util::prof::probe(crate::util::prof::POLL);
            engine.poll_hot(now);
            drop(_p);
            if engine.has_backlog() {
                engine.poll_backlog(now);
            }
        }
        if check_notices {
            last_notice_scan = now;
            process_notices(&mut engine, &notifications_dir, now);
        }
        engine.advance(now);
        engine.ledger.maybe_save(now);

        // Official allowance: rare, background, only while Codex work is visible.
        let view = {
            let _p = crate::util::prof::probe(crate::util::prof::VIEW);
            engine.view(now)
        };
        let codex_visible = view.tasks.iter().any(|t| t.source == engine::Source::Codex);
        // DeepSeek balance: only with a user-provided key; every 10 min while
        // DSH was used in the last hour, otherwise hourly.
        if !balance_running && now >= balance_next {
            let active = engine.dsh_recently_active(now);
            balance_next = now + if active { 10 * 60 * 1000 } else { 60 * 60 * 1000 };
            if let Some(key) = crate::secrets::load_deepseek_key(&paths) {
                balance_running = true;
                let tx = self_tx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(Command::DshBalance(engine::dsh_balance::fetch(&key)));
                });
            }
        }
        if !official_running && now >= official_next && codex_visible {
            official_running = true;
            official_next = now + 10 * 60 * 1000;
            let tx = self_tx.clone();
            let home = paths.codex_home.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Command::Official(allowance::read(home)));
            });
        }

        let locale = catalog.settings_locale(&engine.settings.language);
        let quota_value = quota.build(&engine.settings, &locale, view.display.as_ref());
        registry::write(&registry_path, &engine.registry_tasks(now), &quota_value, &mut registry_cache, now);

        let differs = last_view.as_ref().is_none_or(|previous| view_changed(previous, &view));
        if differs {
            if last_view.as_ref().is_none_or(|p| p.tasks.len() != view.tasks.len() || p.overall_status != view.overall_status) {
                crate::util::log::write(format!(
                    "view: tasks={} overall={} display={} scanned={}",
                    view.tasks.len(),
                    view.overall_status,
                    view.display.is_some(),
                    view.initial_scan_complete
                ));
            }
            *shared.view.lock().unwrap() = Arc::new(view.clone());
            last_view = Some(view);
            responsive_until = responsive_until.max(now + 2000);
            notify();
        }
    }
}

fn view_changed(previous: &View, view: &View) -> bool {
    previous.tasks != view.tasks
        || previous.overall_status != view.overall_status
        || previous.display != view.display
        || previous.account != view.account
        || previous.paused != view.paused
        || previous.initial_scan_complete != view.initial_scan_complete
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publishes_account_updates_without_visible_tasks() {
        let pending = View::default();
        let mut ready = pending.clone();
        ready.account.dsh_balance = Some("CNY 12.34".into());
        ready.account.dsh_at = Some(1000);
        assert!(view_changed(&pending, &ready));
        assert!(!view_changed(&ready, &ready));

        let mut failed = ready.clone();
        failed.account.dsh_failure = Some("network");
        assert!(view_changed(&ready, &failed));
        assert!(view_changed(&failed, &ready));
        assert!(view_changed(&ready, &pending));

        let mut refreshed = ready.clone();
        refreshed.account.dsh_at = Some(2000);
        assert!(view_changed(&ready, &refreshed));

        let mut codex = pending.clone();
        codex.account.codex_five_hour_remaining = Some(61.0);
        codex.account.codex_weekly_remaining = Some(78.0);
        codex.account.codex_at = Some(3000);
        assert!(view_changed(&pending, &codex));
    }
}

fn process_notices(engine: &mut Engine, dir: &std::path::Path, now: i64) {
    for incoming in notices::read_inbox(dir) {
        if !engine.settings.notices.enabled {
            crate::util::fsx::remove_quietly(&incoming.path);
            continue;
        }
        let recipe = if engine.settings.notices.permission == "expressive" {
            incoming.animation.as_ref().and_then(|a| notices::parse_recipe(a, &engine.settings.notices.color))
        } else {
            None
        };
        if engine.accept_notice(incoming.task_number, incoming.agent, incoming.message.clone(), recipe, now).is_some()
            || now - incoming.created_ms > 60_000
        {
            crate::util::fsx::remove_quietly(&incoming.path);
        }
    }
}
