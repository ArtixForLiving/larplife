use crate::config::Config;
use crate::discord_client::DiscordClient;
use crate::logger::{log_error, log_info};
use crate::window_reader::{is_process_running, read_foreground_window};
use crate::{current_unix_seconds, PresenceTracker, TrackerAction};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerCommand {
    ReloadConfig,
    TogglePause,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerEvent {
    ConfigReloaded {
        success: bool,
        app_count: usize,
        error_message: Option<String>,
    },
    StatusChanged {
        paused: bool,
        active_app: Option<String>,
    },
}

pub fn run_worker(
    config_path: PathBuf,
    cmd_rx: Receiver<WorkerCommand>,
    event_tx: Sender<WorkerEvent>,
) {
    let mut config = Config::load_or_create(&config_path).unwrap_or_else(|_| Config::default_presets());
    let mut discord = DiscordClient::new();
    let mut tracker = PresenceTracker::new();
    let mut paused = false;

    log_info("Worker loop started.");

    loop {
        let poll_interval = if paused {
            Duration::from_millis(500)
        } else {
            Duration::from_secs(config.check_interval_seconds.max(1))
        };

        match cmd_rx.recv_timeout(poll_interval) {
            Ok(WorkerCommand::Shutdown) => {
                log_info("Worker received shutdown signal. Disconnecting Discord RPC.");
                discord.disconnect();
                break;
            }
            Ok(WorkerCommand::TogglePause) => {
                paused = !paused;
                if paused {
                    let _ = discord.clear_presence();
                    log_info("Activity tracking paused by user.");
                } else {
                    log_info("Activity tracking resumed by user.");
                }
                let active_app = if paused {
                    None
                } else {
                    tracker.active.as_ref().map(|a| a.process_name.clone())
                };
                let _ = event_tx.send(WorkerEvent::StatusChanged {
                    paused,
                    active_app,
                });
            }
            Ok(WorkerCommand::ReloadConfig) => {
                match Config::load_or_create(&config_path) {
                    Ok(new_cfg) => {
                        let count = new_cfg.apps.len();
                        config = new_cfg;
                        log_info(&format!("Config reloaded successfully with {} apps.", count));
                        let _ = event_tx.send(WorkerEvent::ConfigReloaded {
                            success: true,
                            app_count: count,
                            error_message: None,
                        });
                    }
                    Err(err) => {
                        let msg = err.to_string();
                        log_error(&format!("Config reload failed: {}", msg));
                        let _ = event_tx.send(WorkerEvent::ConfigReloaded {
                            success: false,
                            app_count: 0,
                            error_message: Some(msg),
                        });
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if !paused {
                    let reading = read_foreground_window();
                    let now = Instant::now();
                    let unix_secs = current_unix_seconds();

                    let action = tracker.step(&config, reading, now, unix_secs, is_process_running);
                    match action {
                        TrackerAction::Set(payload) => {
                            let _ = discord.update_presence(&payload, now);
                        }
                        TrackerAction::Clear => {
                            let _ = discord.clear_presence();
                        }
                        TrackerAction::None => {}
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                log_info("Command channel disconnected. Exiting worker.");
                discord.disconnect();
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn worker_handles_shutdown_instantly() {
        let (cmd_tx, cmd_rx) = channel();
        let (event_tx, _event_rx) = channel();

        let handle = thread::spawn(move || {
            run_worker(Config::default_path(), cmd_rx, event_tx);
        });

        cmd_tx.send(WorkerCommand::Shutdown).expect("send shutdown");
        handle.join().expect("worker should exit cleanly without timeout");
    }

    #[test]
    fn worker_handles_toggle_pause() {
        let (cmd_tx, cmd_rx) = channel();
        let (event_tx, event_rx) = channel();

        let handle = thread::spawn(move || {
            run_worker(Config::default_path(), cmd_rx, event_tx);
        });

        cmd_tx.send(WorkerCommand::TogglePause).expect("send pause");
        let event = event_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("should receive pause status event");

        assert_eq!(
            event,
            WorkerEvent::StatusChanged {
                paused: true,
                active_app: None,
            }
        );

        cmd_tx.send(WorkerCommand::Shutdown).expect("send shutdown");
        handle.join().expect("clean exit");
    }

    #[test]
    fn worker_handles_reload_with_custom_path() {
        let (cmd_tx, cmd_rx) = channel();
        let (event_tx, event_rx) = channel();
        let path = std::env::temp_dir().join("larplife_worker_reload_test.json");
        let cfg = Config::default_presets();
        let _ = cfg.save_to_file(&path);

        let worker_path = path.clone();
        let handle = thread::spawn(move || {
            run_worker(worker_path, cmd_rx, event_tx);
        });

        cmd_tx.send(WorkerCommand::ReloadConfig).expect("send reload");
        let event = event_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("should receive reload event");

        assert!(matches!(event, WorkerEvent::ConfigReloaded { success: true, .. }));

        cmd_tx.send(WorkerCommand::Shutdown).expect("send shutdown");
        handle.join().expect("clean exit");
        let _ = std::fs::remove_file(&path);
    }
}
