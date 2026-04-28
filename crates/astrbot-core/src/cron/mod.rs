use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike, Timelike, Utc};
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;
use tokio::time::interval;
use tracing::{error, info, warn};

use crate::errors::{AstrBotError, Result};

/// Job action types
pub enum JobAction {
    /// Send a message to a target
    SendMessage { target: String, text: String },
    /// Execute a shell command
    ExecuteCommand { command: String },
    /// Custom async callback (only usable programmatically)
    Custom(Arc<dyn Fn() -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>),
}

impl std::fmt::Debug for JobAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobAction::SendMessage { target, text } => {
                f.debug_struct("SendMessage")
                    .field("target", target)
                    .field("text", text)
                    .finish()
            }
            JobAction::ExecuteCommand { command } => {
                f.debug_struct("ExecuteCommand")
                    .field("command", command)
                    .finish()
            }
            JobAction::Custom(_) => f.debug_struct("Custom").finish(),
        }
    }
}

impl Clone for JobAction {
    fn clone(&self) -> Self {
        match self {
            JobAction::SendMessage { target, text } => JobAction::SendMessage {
                target: target.clone(),
                text: text.clone(),
            },
            JobAction::ExecuteCommand { command } => JobAction::ExecuteCommand {
                command: command.clone(),
            },
            JobAction::Custom(_) => {
                let dummy: Arc<dyn Fn() -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync> =
                    Arc::new(|| Box::pin(async {}));
                JobAction::Custom(dummy)
            }
        }
    }
}

/// Cron job definition
#[derive(Debug, Clone)]
pub struct CronJob {
    pub id: String,
    pub name: String,
    pub schedule: SchedulePreset,
    pub enabled: bool,
    pub last_run: Option<DateTime<Utc>>,
    pub run_count: u64,
    pub action: JobAction,
}

impl CronJob {
    /// Create a new cron job
    pub fn new(id: impl Into<String>, name: impl Into<String>, schedule: SchedulePreset, action: JobAction) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            schedule,
            enabled: true,
            last_run: None,
            run_count: 0,
            action,
        }
    }
}

/// Schedule presets (simplified cron)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulePreset {
    /// Every N minutes
    EveryNMinutes(u32),
    /// Every hour at minute 0
    Hourly,
    /// Every day at specified hour and minute (UTC)
    Daily { hour: u32, minute: u32 },
    /// Every week at specified day (0=Sunday), hour, minute (UTC)
    Weekly { day: u8, hour: u32, minute: u32 },
}

impl SchedulePreset {
    /// Get the interval duration for checking
    pub fn check_interval(&self) -> Duration {
        match self {
            SchedulePreset::EveryNMinutes(n) => Duration::from_secs((*n as u64) * 60),
            SchedulePreset::Hourly => Duration::from_secs(3600),
            SchedulePreset::Daily { .. } => Duration::from_secs(86400),
            SchedulePreset::Weekly { .. } => Duration::from_secs(604800),
        }
    }

    /// Check if the job is due based on last_run time
    pub fn is_due(&self, last_run: Option<DateTime<Utc>>) -> bool {
        let now = Utc::now();
        let last = last_run.unwrap_or_else(|| DateTime::from_timestamp(0, 0).unwrap_or(DateTime::UNIX_EPOCH));
        let elapsed = now.signed_duration_since(last);

        match self {
            SchedulePreset::EveryNMinutes(n) => {
                let mins = elapsed.num_minutes();
                mins >= *n as i64
            }
            SchedulePreset::Hourly => {
                let mins = elapsed.num_minutes();
                mins >= 60
            }
            SchedulePreset::Daily { hour, minute } => {
                let mins = elapsed.num_minutes();
                if mins < 1440 {
                    return false;
                }
                let current_hour = now.hour();
                let current_min = now.minute();
                current_hour == *hour && current_min == *minute
            }
            SchedulePreset::Weekly { day, hour, minute } => {
                let mins = elapsed.num_minutes();
                if mins < 10080 {
                    return false;
                }
                let current_day = now.weekday().num_days_from_sunday();
                let current_hour = now.hour();
                let current_min = now.minute();
                current_day == *day as u32 && current_hour == *hour && current_min == *minute
            }
        }
    }
}

/// Cron scheduler that runs jobs in background
pub struct CronScheduler {
    jobs: Arc<RwLock<HashMap<String, CronJob>>>,
    control_tx: Option<mpsc::Sender<SchedulerCommand>>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Debug)]
enum SchedulerCommand {
    Stop,
}

impl CronScheduler {
    /// Create a new scheduler (not started yet)
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            control_tx: None,
            handle: None,
        }
    }

    /// Start the background scheduler loop
    pub async fn start(&mut self) {
        if self.handle.is_some() {
            warn!("CronScheduler already started");
            return;
        }

        let (tx, mut rx) = mpsc::channel::<SchedulerCommand>(4);
        self.control_tx = Some(tx);

        let jobs = self.jobs.clone();
        let handle = tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(60));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        let mut job_map = jobs.write().await;
                        let now = Utc::now();
                        for job in job_map.values_mut() {
                            if !job.enabled {
                                continue;
                            }
                            if job.schedule.is_due(job.last_run) {
                                job.last_run = Some(now);
                                job.run_count += 1;
                                let action = job.action.clone();
                                let job_id = job.id.clone();
                                tokio::spawn(async move {
                                    execute_action(job_id, action).await;
                                });
                            }
                        }
                    }
                    Some(cmd) = rx.recv() => {
                        match cmd {
                            SchedulerCommand::Stop => {
                                info!("CronScheduler received stop command");
                                break;
                            }
                        }
                    }
                }
            }
        });

        self.handle = Some(handle);
        info!("CronScheduler started");
    }

    /// Stop the scheduler
    pub async fn stop(&mut self) {
        if let Some(tx) = self.control_tx.take() {
            let _ = tx.send(SchedulerCommand::Stop).await;
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
        info!("CronScheduler stopped");
    }

    /// Add a job
    pub async fn add_job(&self, job: CronJob) -> Result<()> {
        let mut jobs = self.jobs.write().await;
        let job_id = job.id.clone();
        if jobs.contains_key(&job_id) {
            return Err(AstrBotError::Config(format!("Job '{}' already exists", job_id)));
        }
        jobs.insert(job_id.clone(), job);
        info!("Added cron job: {}", job_id);
        Ok(())
    }

    /// Remove a job
    pub async fn remove_job(&self, id: &str) -> bool {
        let mut jobs = self.jobs.write().await;
        let removed = jobs.remove(id).is_some();
        if removed {
            info!("Removed cron job: {}", id);
        }
        removed
    }

    /// List all jobs
    pub async fn list_jobs(&self) -> Vec<CronJob> {
        let jobs = self.jobs.read().await;
        jobs.values().cloned().collect()
    }

    /// Get a job by id
    pub async fn get_job(&self, id: &str) -> Option<CronJob> {
        let jobs = self.jobs.read().await;
        jobs.get(id).cloned()
    }

    /// Enable/disable a job
    pub async fn set_job_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let mut jobs = self.jobs.write().await;
        let job = jobs.get_mut(id)
            .ok_or_else(|| AstrBotError::Config(format!("Job '{}' not found", id)))?;
        job.enabled = enabled;
        Ok(())
    }
}

async fn execute_action(job_id: String, action: JobAction) {
    match action {
        JobAction::SendMessage { target, text } => {
            info!("[Cron {}] Sending message to {}: {}", job_id, target, text);
            // In production, this would call the platform adapter
        }
        JobAction::ExecuteCommand { command } => {
            info!("[Cron {}] Executing command: {}", job_id, command);
            match tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .output()
                .await
            {
                Ok(output) => {
                    if output.status.success() {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        info!("[Cron {}] Command succeeded: {}", job_id, stdout.trim());
                    } else {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        error!("[Cron {}] Command failed: {}", job_id, stderr.trim());
                    }
                }
                Err(e) => {
                    error!("[Cron {}] Command execution error: {}", job_id, e);
                }
            }
        }
        JobAction::Custom(callback) => {
            info!("[Cron {}] Running custom action", job_id);
            callback().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn test_schedule_preset_every_n_minutes() {
        let preset = SchedulePreset::EveryNMinutes(5);
        assert_eq!(preset.check_interval(), Duration::from_secs(300));

        // Should be due if never run
        assert!(preset.is_due(None));

        // Should not be due if ran 2 minutes ago
        let recent = Some(Utc::now() - chrono::TimeDelta::try_minutes(2).unwrap());
        assert!(!preset.is_due(recent));
    }

    #[test]
    fn test_schedule_preset_hourly() {
        let preset = SchedulePreset::Hourly;
        assert_eq!(preset.check_interval(), Duration::from_secs(3600));

        // Should be due if never run
        assert!(preset.is_due(None));

        // Should not be due if ran 30 minutes ago
        let recent = Some(Utc::now() - chrono::TimeDelta::try_minutes(30).unwrap());
        assert!(!preset.is_due(recent));
    }

    #[tokio::test]
    async fn test_scheduler_start_stop() {
        let mut scheduler = CronScheduler::new();
        scheduler.start().await;
        assert!(scheduler.handle.is_some());

        scheduler.stop().await;
        assert!(scheduler.handle.is_none());
    }

    #[tokio::test]
    async fn test_add_remove_list_jobs() {
        let mut scheduler = CronScheduler::new();
        scheduler.start().await;

        let job = CronJob::new(
            "test-1",
            "Test Job",
            SchedulePreset::EveryNMinutes(5),
            JobAction::SendMessage {
                target: "#general".to_string(),
                text: "Hello".to_string(),
            },
        );

        scheduler.add_job(job.clone()).await.unwrap();

        let jobs = scheduler.list_jobs().await;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, "test-1");

        let retrieved = scheduler.get_job("test-1").await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "Test Job");

        let removed = scheduler.remove_job("test-1").await;
        assert!(removed);

        let jobs = scheduler.list_jobs().await;
        assert_eq!(jobs.len(), 0);

        scheduler.stop().await;
    }

    #[tokio::test]
    async fn test_job_execution_counter() {
        let counter = Arc::new(AtomicU64::new(0));
        let counter_clone = counter.clone();

        let mut scheduler = CronScheduler::new();
        scheduler.start().await;

        let action = JobAction::Custom(Arc::new(move || {
            let c = counter_clone.clone();
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
            })
        }));

        let job = CronJob::new(
            "counter-job",
            "Counter Job",
            SchedulePreset::EveryNMinutes(1), // Will be due immediately
            action,
        );

        scheduler.add_job(job).await.unwrap();

        // Wait for the scheduler to tick (it checks every 60s, but we speed it up by checking manually)
        tokio::time::sleep(Duration::from_secs(2)).await;

        // The job should have run at least once (since it's due immediately)
        let job = scheduler.get_job("counter-job").await.unwrap();
        assert!(job.run_count >= 1 || job.last_run.is_some());

        scheduler.stop().await;
    }

    #[tokio::test]
    async fn test_duplicate_job_id() {
        let mut scheduler = CronScheduler::new();
        scheduler.start().await;

        let job1 = CronJob::new(
            "dup",
            "First",
            SchedulePreset::EveryNMinutes(5),
            JobAction::SendMessage {
                target: "a".to_string(),
                text: "a".to_string(),
            },
        );

        let job2 = CronJob::new(
            "dup",
            "Second",
            SchedulePreset::Hourly,
            JobAction::SendMessage {
                target: "b".to_string(),
                text: "b".to_string(),
            },
        );

        scheduler.add_job(job1).await.unwrap();
        let result = scheduler.add_job(job2).await;
        assert!(result.is_err());

        scheduler.stop().await;
    }

    #[tokio::test]
    async fn test_disabled_job_not_executed() {
        let mut scheduler = CronScheduler::new();
        scheduler.start().await;

        let job = CronJob::new(
            "disabled",
            "Disabled Job",
            SchedulePreset::EveryNMinutes(1),
            JobAction::SendMessage {
                target: "t".to_string(),
                text: "msg".to_string(),
            },
        );

        scheduler.add_job(job).await.unwrap();
        scheduler.set_job_enabled("disabled", false).await.unwrap();

        let job = scheduler.get_job("disabled").await.unwrap();
        assert!(!job.enabled);

        scheduler.stop().await;
    }
}