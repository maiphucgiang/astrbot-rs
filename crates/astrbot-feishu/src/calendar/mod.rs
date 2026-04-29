//! Feishu calendar integration: events, reminders, free-busy queries

use chrono::{DateTime, Duration, Utc};
use reqwest::Method;
use serde_json::json;
use tracing::{debug, info, warn};

use crate::{auth::FeishuAuth, CalendarEventData, FeishuError, FeishuUser, Result};

/// Calendar client for Feishu
pub struct CalendarClient {
    auth: FeishuAuth,
}

/// Reminder configuration
#[derive(Clone, Debug)]
pub struct ReminderConfig {
    pub before_minutes: Vec<i32>,
    pub default_before: i32,
}

impl Default for ReminderConfig {
    fn default() -> Self {
        Self {
            before_minutes: vec![15, 60, 1440],
            default_before: 15,
        }
    }
}

/// Event filter for listing
#[derive(Clone, Debug, Default)]
pub struct EventFilter {
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub calendar_id: Option<String>,
}

impl CalendarClient {
    pub fn new(auth: FeishuAuth) -> Self {
        Self { auth }
    }

    /// Get user's primary calendar ID
    pub async fn primary_calendar(&self, _user_open_id: &str) -> Result<String> {
        let path = "/calendar/v4/calendars/primary";
        let req = self.auth.auth_request(Method::GET, path).await?;
        let resp = req.send().await.map_err(FeishuError::Http)?;

        let api_resp: crate::ApiResponse<serde_json::Value> =
            resp.json().await.map_err(FeishuError::Http)?;

        if api_resp.code != 0 || api_resp.data.is_none() {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        let data = api_resp.data.unwrap();
        let calendar_id = data
            .get("calendars")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|cal| cal.get("calendar_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("primary")
            .to_string();

        Ok(calendar_id)
    }

    /// List events in a calendar
    pub async fn list_events(
        &self,
        calendar_id: &str,
        filter: &EventFilter,
    ) -> Result<Vec<CalendarEventData>> {
        let mut path = format!(
            "/calendar/v4/calendars/{}/events?page_size=500",
            calendar_id
        );

        if let Some(start) = filter.start_time {
            path.push_str(&format!("&start_time={}", start.to_rfc3339()));
        }
        if let Some(end) = filter.end_time {
            path.push_str(&format!("&end_time={}", end.to_rfc3339()));
        }

        let req = self.auth.auth_request(Method::GET, &path).await?;
        let resp = req.send().await.map_err(FeishuError::Http)?;

        let api_resp: crate::ApiResponse<serde_json::Value> =
            resp.json().await.map_err(FeishuError::Http)?;

        if api_resp.code != 0 || api_resp.data.is_none() {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        let data = api_resp.data.unwrap();
        let items = data
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let events: Vec<CalendarEventData> = items
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect();

        info!(
            "Listed {} events from calendar {}",
            events.len(),
            calendar_id
        );
        Ok(events)
    }

    /// Create a calendar event
    pub async fn create_event(
        &self,
        calendar_id: &str,
        event: &CalendarEventData,
        reminders: &ReminderConfig,
    ) -> Result<String> {
        let body = json!({
            "summary": event.summary,
            "description": event.description,
            "start": {
                "date_time": event.start_time.to_rfc3339(),
                "timezone": "Asia/Shanghai"
            },
            "end": {
                "date_time": event.end_time.to_rfc3339(),
                "timezone": "Asia/Shanghai"
            },
            "location": event.location,
            "attendee_ability": "can_see_others",
            "free_busy_status": "busy",
            "reminders": reminders.before_minutes.iter().map(|m| {
                json!({ "minutes": m })
            }).collect::<Vec<_>>(),
            "attendees": event.attendees.iter().map(|id| {
                json!({ "type": "user", "user_id": id })
            }).collect::<Vec<_>>(),
        });

        let path = format!("/calendar/v4/calendars/{}/events", calendar_id);
        let req = self.auth.auth_request(Method::POST, &path).await?;
        let resp = req.json(&body).send().await.map_err(FeishuError::Http)?;

        let api_resp: crate::ApiResponse<serde_json::Value> =
            resp.json().await.map_err(FeishuError::Http)?;

        if api_resp.code != 0 || api_resp.data.is_none() {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        let event_id = api_resp
            .data
            .and_then(|d| d.get("event").cloned())
            .and_then(|e| {
                e.get("event_id")
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
            })
            .unwrap_or_default();

        info!("Created event {} in calendar {}", event_id, calendar_id);
        Ok(event_id)
    }

    /// Delete an event
    pub async fn delete_event(&self, calendar_id: &str, event_id: &str) -> Result<()> {
        let path = format!("/calendar/v4/calendars/{}/events/{}", calendar_id, event_id);
        let req = self.auth.auth_request(Method::DELETE, &path).await?;
        let resp = req.send().await.map_err(FeishuError::Http)?;

        let api_resp: crate::ApiResponse<serde_json::Value> =
            resp.json().await.map_err(FeishuError::Http)?;

        if api_resp.code != 0 {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        info!("Deleted event {} from calendar {}", event_id, calendar_id);
        Ok(())
    }

    /// Check free/busy for users
    pub async fn check_freebusy(
        &self,
        user_ids: &[String],
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> Result<Vec<FreeBusySlot>> {
        let body = json!({
            "user_ids": user_ids,
            "start_time": start_time.to_rfc3339(),
            "end_time": end_time.to_rfc3339(),
            "time_zone": "Asia/Shanghai",
            "capacity": 100,
        });

        let path = "/calendar/v4/freebusy/batch_get";
        let req = self.auth.auth_request(Method::POST, path).await?;
        let resp = req.json(&body).send().await.map_err(FeishuError::Http)?;

        let api_resp: crate::ApiResponse<serde_json::Value> =
            resp.json().await.map_err(FeishuError::Http)?;

        if api_resp.code != 0 || api_resp.data.is_none() {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        let data = api_resp.data.unwrap();
        let slots = data
            .get("freebusy_list")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let freebusy: Vec<FreeBusySlot> = slots
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect();

        Ok(freebusy)
    }

    /// Get upcoming events for a user (next N hours)
    pub async fn upcoming_events(
        &self,
        user_open_id: &str,
        hours: i64,
    ) -> Result<Vec<CalendarEventData>> {
        let calendar_id = self.primary_calendar(user_open_id).await?;
        let now = Utc::now();
        let filter = EventFilter {
            start_time: Some(now),
            end_time: Some(now + Duration::hours(hours)),
            calendar_id: Some(calendar_id.clone()),
        };

        self.list_events(&calendar_id, &filter).await
    }

    /// Format events as a reminder text for bot usage
    pub fn format_reminder(events: &[CalendarEventData]) -> String {
        if events.is_empty() {
            return "📅 近期没有日程".to_string();
        }

        let mut text = format!("📅 你有 {} 个 upcoming 日程:\n", events.len());
        for (i, evt) in events.iter().enumerate() {
            text.push_str(&format!(
                "\n{}. {}\n   🕐 {} - {}\n",
                i + 1,
                evt.summary,
                evt.start_time.format("%m-%d %H:%M"),
                evt.end_time.format("%m-%d %H:%M")
            ));
            if let Some(loc) = &evt.location {
                text.push_str(&format!("   📍 {}\n", loc));
            }
        }
        text
    }
}

/// Free/busy slot for a user
#[derive(Clone, Debug, serde::Deserialize)]
pub struct FreeBusySlot {
    pub user_id: String,
    #[serde(default)]
    pub start_time: Option<DateTime<Utc>>,
    #[serde(default)]
    pub end_time: Option<DateTime<Utc>>,
    #[serde(default)]
    pub free_busy_type: Option<String>, // "busy" or "free"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reminder_config_default() {
        let cfg = ReminderConfig::default();
        assert_eq!(cfg.default_before, 15);
        assert_eq!(cfg.before_minutes, vec![15, 60, 1440]);
    }

    #[test]
    fn test_format_reminder_empty() {
        let text = CalendarClient::format_reminder(&[]);
        assert!(text.contains("没有日程"));
    }
}
