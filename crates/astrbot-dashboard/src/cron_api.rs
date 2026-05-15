//! Cron REST API handlers — expose cron scheduler via HTTP endpoints.

use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::app_state::AppState;

// ── Request DTOs ──

#[derive(Debug, Deserialize)]
pub struct CreateCronJobRequest {
    pub id: String,
    pub name: Option<String>,
    pub schedule: String, // "every_5_minutes" | "hourly" | "daily_09:00" | "weekly_0_09:00"
    pub action_type: String, // "send_message" | "execute_command" | "custom"
    pub target: Option<String>,
    pub text: Option<String>,
    pub command: Option<String>,
}

fn parse_schedule_preset(s: &str) -> Option<astrbot_core::cron::SchedulePreset> {
    match s {
        "hourly" => Some(astrbot_core::cron::SchedulePreset::Hourly),
        s if s.starts_with("every_") && s.ends_with("_minutes") => {
            let num = s
                .trim_start_matches("every_")
                .trim_end_matches("_minutes")
                .parse::<u32>()
                .ok()?;
            Some(astrbot_core::cron::SchedulePreset::EveryNMinutes(num))
        }
        s if s.starts_with("daily_") => {
            let parts: Vec<&str> = s.trim_start_matches("daily_").split(':').collect();
            if parts.len() == 2 {
                let hour = parts[0].parse::<u32>().ok()?;
                let minute = parts[1].parse::<u32>().ok()?;
                return Some(astrbot_core::cron::SchedulePreset::Daily { hour, minute });
            }
            None
        }
        s if s.starts_with("weekly_") => {
            let parts: Vec<&str> = s.trim_start_matches("weekly_").split('_').collect();
            if parts.len() == 3 {
                let day = parts[0].parse::<u8>().ok()?;
                let hour = parts[1].parse::<u32>().ok()?;
                let minute = parts[2].parse::<u32>().ok()?;
                return Some(astrbot_core::cron::SchedulePreset::Weekly { day, hour, minute });
            }
            None
        }
        _ => None,
    }
}

fn build_job_action(req: &CreateCronJobRequest) -> Result<astrbot_core::cron::JobAction, String> {
    match req.action_type.as_str() {
        "send_message" => {
            let target = req
                .target
                .clone()
                .ok_or("target required for send_message")?;
            let text = req.text.clone().ok_or("text required for send_message")?;
            Ok(astrbot_core::cron::JobAction::SendMessage { target, text })
        }
        "execute_command" => {
            let command = req
                .command
                .clone()
                .ok_or("command required for execute_command")?;
            Ok(astrbot_core::cron::JobAction::ExecuteCommand { command })
        }
        "custom" => {
            let dummy: Arc<
                dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
                    + Send
                    + Sync,
            > = Arc::new(|| Box::pin(async {}));
            Ok(astrbot_core::cron::JobAction::Custom(dummy))
        }
        _ => Err(format!("unknown action_type: {}", req.action_type)),
    }
}

// ── Handlers ──

pub async fn list_cron_jobs(State(state): State<AppState>) -> Json<Value> {
    if let Some(ref cs) = state.cron_scheduler {
        let jobs = cs.list_jobs().await;
        let items: Vec<Value> = jobs
            .into_iter()
            .map(|job| {
                json!({
                    "id": job.id,
                    "name": job.name,
                    "schedule": format!("{:?}", job.schedule),
                    "enabled": job.enabled,
                    "run_count": job.run_count,
                    "last_run": job.last_run.map(|dt| dt.to_rfc3339()),
                })
            })
            .collect();
        return Json(json!({
            "jobs": items,
            "total": items.len(),
        }));
    }
    Json(json!({
        "jobs": [],
        "total": 0,
        "note": "cron_scheduler not available",
    }))
}

pub async fn create_cron_job(
    State(state): State<AppState>,
    Json(req): Json<CreateCronJobRequest>,
) -> Json<Value> {
    if let Some(ref cs) = state.cron_scheduler {
        let schedule = match parse_schedule_preset(&req.schedule) {
            Some(s) => s,
            None => {
                return Json(json!({
                    "success": false,
                    "error": format!("Invalid schedule preset: '{}'. Supported: hourly, every_N_minutes, daily_HH:MM, weekly_D_HH:MM", req.schedule),
                }));
            }
        };
        let action = match build_job_action(&req) {
            Ok(a) => a,
            Err(e) => {
                return Json(json!({
                    "success": false,
                    "error": e,
                }));
            }
        };
        let job = astrbot_core::cron::CronJob::new(
            &req.id,
            req.name.as_deref().unwrap_or(&req.id),
            schedule,
            action,
        );
        match cs.add_job(job).await {
            Ok(_) => Json(json!({
                "success": true,
                "job_id": req.id,
                "schedule": req.schedule,
                "name": req.name,
            })),
            Err(e) => Json(json!({
                "success": false,
                "error": format!("{}", e),
            })),
        }
    } else {
        Json(json!({
            "success": false,
            "error": "cron_scheduler not available",
        }))
    }
}

pub async fn delete_cron_job(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    if let Some(ref cs) = state.cron_scheduler {
        let removed = cs.remove_job(&id).await;
        return Json(json!({
            "success": removed,
            "job_id": id,
        }));
    }
    Json(json!({
        "success": false,
        "error": "cron_scheduler not available",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Path, State};
    use axum::Json;
    use serde_json::json;

    fn test_state_with_cron() -> AppState {
        let pm = Arc::new(tokio::sync::RwLock::new(
            astrbot_plugin::PluginManager::new(std::path::PathBuf::from("data/plugins")),
        ));
        let pvm = Arc::new(tokio::sync::RwLock::new(
            astrbot_provider::client::ProviderManager::new(),
        ));
        let mut state = AppState::new(pm, pvm);
        state.cron_scheduler = Some(Arc::new(astrbot_core::cron::CronScheduler::new()));
        state
    }

    #[tokio::test]
    async fn test_list_cron_jobs_empty() {
        let state = test_state_with_cron();
        let result = list_cron_jobs(State(state)).await;
        let json = result.0;
        assert!(json["jobs"].is_array());
        assert_eq!(json["total"], 0);
    }

    #[tokio::test]
    async fn test_create_and_list_cron_job() {
        let state = test_state_with_cron();
        let req = CreateCronJobRequest {
            id: "job_test".to_string(),
            name: Some("hourly_task".to_string()),
            schedule: "hourly".to_string(),
            action_type: "execute_command".to_string(),
            target: None,
            text: None,
            command: Some("echo hello".to_string()),
        };
        let result = create_cron_job(State(state.clone()), Json(req)).await;
        let json = result.0;
        assert_eq!(json["success"], true);
        let job_id = json["job_id"].as_str().unwrap().to_string();
        let result = list_cron_jobs(State(state)).await;
        let json = result.0;
        assert_eq!(json["total"], 1);
        assert_eq!(json["jobs"][0]["id"], job_id);
    }

    #[tokio::test]
    async fn test_delete_cron_job() {
        let state = test_state_with_cron();
        let req = CreateCronJobRequest {
            id: "del_me".to_string(),
            name: None,
            schedule: "every_5_minutes".to_string(),
            action_type: "send_message".to_string(),
            target: Some("user_123".to_string()),
            text: Some("hello".to_string()),
            command: None,
        };
        let result = create_cron_job(State(state.clone()), Json(req)).await;
        let json = result.0;
        let job_id = json["job_id"].as_str().unwrap().to_string();
        let result = delete_cron_job(State(state), Path(job_id.clone())).await;
        let json = result.0;
        assert_eq!(json["success"], true);
        assert_eq!(json["job_id"], job_id);
    }

    #[tokio::test]
    async fn test_delete_nonexistent_job() {
        let state = test_state_with_cron();
        let result = delete_cron_job(State(state), Path("nonexistent".to_string())).await;
        let json = result.0;
        assert_eq!(json["success"], false);
    }

    #[tokio::test]
    async fn test_invalid_schedule() {
        let state = test_state_with_cron();
        let req = CreateCronJobRequest {
            id: "bad".to_string(),
            name: None,
            schedule: "not-a-schedule".to_string(),
            action_type: "custom".to_string(),
            target: None,
            text: None,
            command: None,
        };
        let result = create_cron_job(State(state), Json(req)).await;
        let json = result.0;
        assert_eq!(json["success"], false);
        assert!(json["error"].as_str().unwrap().contains("Invalid schedule"));
    }
}
