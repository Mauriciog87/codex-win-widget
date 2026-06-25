use chrono::{DateTime, Local};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusLevel {
    Good,
    Warning,
    Critical,
    Unknown,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccountInfo {
    pub auth_type: Option<String>,
    pub plan_type: Option<String>,
    pub requires_openai_auth: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LimitWindow {
    pub used_percent: f64,
    pub window_duration_mins: Option<u64>,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreditsInfo {
    pub has_credits: Option<bool>,
    pub unlimited: Option<bool>,
    pub balance: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RateLimitBucket {
    pub limit_id: String,
    pub limit_name: Option<String>,
    pub primary: Option<LimitWindow>,
    pub secondary: Option<LimitWindow>,
    pub credits: Option<CreditsInfo>,
    pub plan_type: Option<String>,
    pub rate_limit_reached_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageSummary {
    pub lifetime_tokens: Option<u64>,
    pub peak_daily_tokens: Option<u64>,
    pub longest_running_turn_sec: Option<u64>,
    pub current_streak_days: Option<u64>,
    pub longest_streak_days: Option<u64>,
    pub latest_daily_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WidgetSnapshot {
    pub account: Option<AccountInfo>,
    pub buckets: Vec<RateLimitBucket>,
    pub reset_credit_count: Option<u64>,
    pub usage: Option<UsageSummary>,
    pub fetched_at: DateTime<Local>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodexCardView {
    pub title: String,
    pub subtitle: String,
    pub primary_value: String,
    pub primary_label: String,
    pub remaining_label: String,
    pub reset_label: String,
    pub credits_label: String,
    pub reset_credits_label: Option<String>,
    pub usage_lines: Vec<(String, String)>,
    pub updated_label: String,
    pub status_message: Option<String>,
    pub progress_percent: f64,
    pub status_level: StatusLevel,
}

impl WidgetSnapshot {
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            account: None,
            buckets: Vec::new(),
            reset_credit_count: None,
            usage: None,
            fetched_at: Local::now(),
            error: Some(message.into()),
        }
    }

    pub fn display_bucket(&self) -> Option<&RateLimitBucket> {
        self.buckets
            .iter()
            .find(|bucket| bucket.limit_id == "codex")
            .or_else(|| self.buckets.first())
    }

    pub fn status_level(&self) -> StatusLevel {
        if self.error.is_some() {
            return StatusLevel::Error;
        }

        let Some(bucket) = self.display_bucket() else {
            return StatusLevel::Unknown;
        };

        if bucket.rate_limit_reached_type.is_some() {
            return StatusLevel::Critical;
        }

        let Some(window) = bucket.display_window() else {
            return StatusLevel::Unknown;
        };

        if window.used_percent >= 90.0 {
            StatusLevel::Critical
        } else if window.used_percent >= 70.0 {
            StatusLevel::Warning
        } else {
            StatusLevel::Good
        }
    }

    pub fn icon_lines(&self) -> (String, String) {
        if self.error.is_some() {
            return ("CX".to_string(), "ERR".to_string());
        }

        let bottom = self
            .display_bucket()
            .and_then(RateLimitBucket::display_window)
            .map(|window| format!("{:.0}", window.used_percent.clamp(0.0, 999.0)))
            .unwrap_or_else(|| "--".to_string());

        ("CX".to_string(), bottom)
    }

    pub fn tooltip(&self) -> String {
        if let Some(error) = &self.error {
            return truncate_tip(&format!("Codex limits not available: {error}"));
        }

        let Some(bucket) = self.display_bucket() else {
            return "Codex limits not available".to_string();
        };

        let Some(window) = bucket.display_window() else {
            return "Codex limits not available".to_string();
        };

        let remaining = (100.0 - window.used_percent).clamp(0.0, 100.0);
        let reset = window
            .resets_at
            .and_then(format_reset_time)
            .unwrap_or_else(|| "reset time unknown".to_string());

        truncate_tip(&format!("Codex {remaining:.0}% remaining, resets {reset}"))
    }

    pub fn status_summary(&self) -> String {
        if let Some(error) = &self.error {
            return format!("Codex limits not available: {error}");
        }

        let plan = self
            .account
            .as_ref()
            .and_then(|account| account.plan_type.as_deref())
            .unwrap_or("plan unknown");

        let Some(bucket) = self.display_bucket() else {
            return format!("Codex limits not available ({plan})");
        };

        let Some(window) = bucket.display_window() else {
            return format!("Codex limits not available ({plan})");
        };

        let reset = window
            .resets_at
            .and_then(format_reset_time)
            .unwrap_or_else(|| "reset time unknown".to_string());

        format!(
            "Codex has {:.0}% remaining, {:.0}% used, resets {reset}, plan {plan}",
            (100.0 - window.used_percent).clamp(0.0, 100.0),
            window.used_percent
        )
    }

    pub fn flyout_rows(&self) -> Vec<(String, String)> {
        let mut rows = Vec::new();

        if let Some(error) = &self.error {
            rows.push(("Status".to_string(), "Not available".to_string()));
            rows.push(("Detail".to_string(), error.clone()));
            rows.push(("Updated".to_string(), format_time(self.fetched_at)));
            return rows;
        }

        if let Some(account) = &self.account {
            rows.push((
                "Sign-in".to_string(),
                account
                    .auth_type
                    .as_deref()
                    .unwrap_or("unknown")
                    .to_string(),
            ));
            rows.push((
                "Plan".to_string(),
                account
                    .plan_type
                    .as_deref()
                    .unwrap_or("unknown")
                    .to_string(),
            ));
        }

        if let Some(bucket) = self.display_bucket() {
            rows.push(("Bucket".to_string(), bucket.display_name()));
            if let Some(window) = bucket.display_window() {
                rows.push(("Used".to_string(), format!("{:.0}%", window.used_percent)));
                rows.push((
                    "Remaining".to_string(),
                    format!("{:.0}%", (100.0 - window.used_percent).clamp(0.0, 100.0)),
                ));
                rows.push((
                    "Reset".to_string(),
                    window
                        .resets_at
                        .and_then(format_reset_time)
                        .unwrap_or_else(|| "unknown".to_string()),
                ));
            }
            if let Some(credits) = &bucket.credits {
                rows.push(("Credits".to_string(), credits.display()));
            }
        }

        if let Some(count) = self.reset_credit_count {
            rows.push(("Reset credits".to_string(), count.to_string()));
        }

        if let Some(usage) = &self.usage {
            if let Some(tokens) = usage.latest_daily_tokens {
                rows.push(("Today tokens".to_string(), compact_number(tokens)));
            }
            if let Some(tokens) = usage.lifetime_tokens {
                rows.push(("Lifetime".to_string(), compact_number(tokens)));
            }
            if let Some(days) = usage.current_streak_days {
                rows.push(("Streak".to_string(), format!("{days} days")));
            }
        }

        rows.push(("Updated".to_string(), format_time(self.fetched_at)));
        rows
    }

    pub fn card_view(&self) -> CodexCardView {
        let updated_label = format!("Updated {}", format_time(self.fetched_at));
        if let Some(error) = &self.error {
            return CodexCardView {
                title: "Codex".to_string(),
                subtitle: "Limits not available".to_string(),
                primary_value: "--".to_string(),
                primary_label: "remaining".to_string(),
                remaining_label: "Used not available".to_string(),
                reset_label: "Reset time unknown".to_string(),
                credits_label: "Credits not available".to_string(),
                reset_credits_label: self
                    .reset_credit_count
                    .map(|count| format!("{count} reset credits")),
                usage_lines: Vec::new(),
                updated_label,
                status_message: Some(error.clone()),
                progress_percent: 0.0,
                status_level: StatusLevel::Error,
            };
        }

        let status_level = self.status_level();
        let plan = self.plan_label();
        let Some(bucket) = self.display_bucket() else {
            return CodexCardView {
                title: "Codex".to_string(),
                subtitle: plan,
                primary_value: "--".to_string(),
                primary_label: "remaining".to_string(),
                remaining_label: "Used not available".to_string(),
                reset_label: "Reset time unknown".to_string(),
                credits_label: "Credits not available".to_string(),
                reset_credits_label: self
                    .reset_credit_count
                    .map(|count| format!("{count} reset credits")),
                usage_lines: self.usage_lines(),
                updated_label,
                status_message: Some("No limit data yet".to_string()),
                progress_percent: 0.0,
                status_level,
            };
        };

        let Some(window) = bucket.display_window() else {
            return CodexCardView {
                title: "Codex".to_string(),
                subtitle: format!("{plan} - {}", bucket.display_name()),
                primary_value: "--".to_string(),
                primary_label: "remaining".to_string(),
                remaining_label: "Used not available".to_string(),
                reset_label: "Reset time unknown".to_string(),
                credits_label: bucket
                    .credits
                    .as_ref()
                    .map(|credits| format!("Credits {}", credits.display()))
                    .unwrap_or_else(|| "Credits not available".to_string()),
                reset_credits_label: self
                    .reset_credit_count
                    .map(|count| format!("{count} reset credits")),
                usage_lines: self.usage_lines(),
                updated_label,
                status_message: Some("No reset window yet".to_string()),
                progress_percent: 0.0,
                status_level,
            };
        };

        let used = window.used_percent.clamp(0.0, 100.0);
        let remaining = (100.0 - used).clamp(0.0, 100.0);
        let reset = window
            .resets_at
            .and_then(format_reset_time)
            .map(|time| format!("Resets {time}"))
            .unwrap_or_else(|| "Reset time unknown".to_string());
        let status_message = bucket
            .rate_limit_reached_type
            .as_ref()
            .map(|value| format!("Limit reached: {value}"));

        CodexCardView {
            title: "Codex".to_string(),
            subtitle: format!("{plan} - {}", bucket.display_name()),
            primary_value: format!("{remaining:.0}%"),
            primary_label: "remaining".to_string(),
            remaining_label: format!("{used:.0}% used"),
            reset_label: reset,
            credits_label: bucket
                .credits
                .as_ref()
                .map(|credits| format!("Credits {}", credits.display()))
                .unwrap_or_else(|| "Credits not available".to_string()),
            reset_credits_label: self
                .reset_credit_count
                .map(|count| format!("{count} reset credits")),
            usage_lines: self.usage_lines(),
            updated_label,
            status_message,
            progress_percent: used,
            status_level,
        }
    }

    fn plan_label(&self) -> String {
        self.account
            .as_ref()
            .and_then(|account| account.plan_type.as_deref())
            .or_else(|| {
                self.display_bucket()
                    .and_then(|bucket| bucket.plan_type.as_deref())
            })
            .filter(|plan| !plan.trim().is_empty())
            .unwrap_or("plan unknown")
            .to_string()
    }

    fn usage_lines(&self) -> Vec<(String, String)> {
        let mut lines = Vec::new();
        if let Some(usage) = &self.usage {
            if let Some(tokens) = usage.latest_daily_tokens {
                lines.push(("Today".to_string(), compact_number(tokens)));
            }
            if let Some(tokens) = usage.lifetime_tokens {
                lines.push(("Lifetime".to_string(), compact_number(tokens)));
            }
            if let Some(days) = usage.current_streak_days {
                lines.push(("Streak".to_string(), format!("{days} days")));
            }
        }
        lines
    }
}

impl RateLimitBucket {
    pub fn display_window(&self) -> Option<&LimitWindow> {
        self.primary.as_ref().or(self.secondary.as_ref())
    }

    pub fn display_name(&self) -> String {
        self.limit_name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(&self.limit_id)
            .to_string()
    }
}

impl CreditsInfo {
    pub fn display(&self) -> String {
        if self.unlimited == Some(true) {
            return "unlimited".to_string();
        }

        match (self.has_credits, self.balance.as_deref()) {
            (Some(true), Some(balance)) if !balance.trim().is_empty() => balance.to_string(),
            (Some(true), _) => "available".to_string(),
            (Some(false), _) => "none".to_string(),
            (_, Some(balance)) if !balance.trim().is_empty() => balance.to_string(),
            _ => "unknown".to_string(),
        }
    }
}

pub fn format_reset_time(timestamp: i64) -> Option<String> {
    DateTime::from_timestamp(timestamp, 0)
        .map(|time| time.with_timezone(&Local).format("%H:%M").to_string())
}

pub fn format_time(time: DateTime<Local>) -> String {
    time.format("%H:%M:%S").to_string()
}

pub fn compact_number(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn truncate_tip(value: &str) -> String {
    const MAX_CHARS: usize = 120;
    if value.chars().count() <= MAX_CHARS {
        return value.to_string();
    }
    value.chars().take(MAX_CHARS - 3).collect::<String>() + "..."
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot_with_percent(used_percent: f64) -> WidgetSnapshot {
        WidgetSnapshot {
            account: Some(AccountInfo {
                auth_type: Some("chatgpt".to_string()),
                plan_type: Some("plus".to_string()),
                requires_openai_auth: Some(true),
            }),
            buckets: vec![RateLimitBucket {
                limit_id: "codex".to_string(),
                limit_name: None,
                primary: Some(LimitWindow {
                    used_percent,
                    window_duration_mins: Some(60),
                    resets_at: None,
                }),
                secondary: None,
                credits: None,
                plan_type: Some("plus".to_string()),
                rate_limit_reached_type: None,
            }],
            reset_credit_count: Some(0),
            usage: None,
            fetched_at: Local::now(),
            error: None,
        }
    }

    #[test]
    fn classifies_status_by_used_percent() {
        assert_eq!(
            snapshot_with_percent(10.0).status_level(),
            StatusLevel::Good
        );
        assert_eq!(
            snapshot_with_percent(70.0).status_level(),
            StatusLevel::Warning
        );
        assert_eq!(
            snapshot_with_percent(90.0).status_level(),
            StatusLevel::Critical
        );
    }

    #[test]
    fn formats_icon_lines_from_percent() {
        assert_eq!(
            snapshot_with_percent(31.4).icon_lines(),
            ("CX".to_string(), "31".to_string())
        );
    }

    #[test]
    fn builds_card_view_from_limit_data() {
        let mut snapshot = snapshot_with_percent(31.4);
        snapshot.buckets[0].credits = Some(CreditsInfo {
            has_credits: Some(true),
            unlimited: Some(false),
            balance: Some("123.45".to_string()),
        });
        snapshot.usage = Some(UsageSummary {
            lifetime_tokens: Some(12_000),
            peak_daily_tokens: None,
            longest_running_turn_sec: None,
            current_streak_days: Some(3),
            longest_streak_days: None,
            latest_daily_tokens: Some(1_200),
        });

        let view = snapshot.card_view();

        assert_eq!(view.title, "Codex");
        assert_eq!(view.subtitle, "plus - codex");
        assert_eq!(view.primary_value, "69%");
        assert_eq!(view.primary_label, "remaining");
        assert_eq!(view.remaining_label, "31% used");
        assert_eq!(view.credits_label, "Credits 123.45");
        assert_eq!(
            view.usage_lines,
            vec![
                ("Today".to_string(), "1.2K".to_string()),
                ("Lifetime".to_string(), "12.0K".to_string()),
                ("Streak".to_string(), "3 days".to_string())
            ]
        );
        assert_eq!(view.progress_percent, 31.4);
        assert_eq!(view.status_level, StatusLevel::Good);
    }

    #[test]
    fn builds_error_card_view() {
        let snapshot = WidgetSnapshot::error("offline");
        let view = snapshot.card_view();

        assert_eq!(view.subtitle, "Limits not available");
        assert_eq!(view.primary_value, "--");
        assert_eq!(view.primary_label, "remaining");
        assert_eq!(view.status_message, Some("offline".to_string()));
        assert_eq!(view.progress_percent, 0.0);
        assert_eq!(view.status_level, StatusLevel::Error);
    }

    #[test]
    fn credits_display_prefers_balance() {
        let credits = CreditsInfo {
            has_credits: Some(true),
            unlimited: Some(false),
            balance: Some("123.45".to_string()),
        };
        assert_eq!(credits.display(), "123.45");
    }

    #[test]
    fn compact_number_uses_short_units() {
        assert_eq!(compact_number(999), "999");
        assert_eq!(compact_number(1_200), "1.2K");
        assert_eq!(compact_number(2_500_000), "2.5M");
    }
}
