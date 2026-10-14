use crate::config::Settings;
use crate::services::workloads::fetch_and_update_all_watched;
use config::ConfigError;
use cron::Schedule;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::{sleep_until, Instant as TokioInstant};



pub async fn scheduler(schedule: &Schedule, max_pages: usize) {
    log::info!("Scheduler started");
    println!("Cron schedule: {}", schedule);
    log::info!("Cron schedule: {}", schedule);
    let mut i = 0;
    for datetime in schedule.upcoming(chrono::Utc) {
        if i < 5 {
            println!("Next scheduled time: {}", datetime);
            i += 1;
        } else {
            break;
        }
    }
    let now = chrono::Utc::now();
    if let Some(next) = schedule.upcoming(chrono::Utc).next() {
        let duration_until_next = (next - now).to_std().expect("Failed to calculate duration");
        println!("Next scheduled time: {:?}", next);
        // Convert std::time::Instant to tokio::time::Instant
        let tokio_now = TokioInstant::now();
        let tokio_future = tokio_now + duration_until_next;
        // Sleep until the next scheduled time
        sleep_until(tokio_future).await;
        // Execute your function
        refresh_all_workloads(max_pages).await;
    }
}

pub async fn run_scheduler(settings: Settings) {
    //Load Scheduler
    let run_at_startup = settings.system.run_at_startup;
    let max_pages = settings.system.registry_max_pages;
    if run_at_startup {
        log::info!("Running full refresh at startup");
        refresh_all_workloads(max_pages).await;
    }
    let schedule_str = settings.system.schedule;
    log::info!("Schedule is {}", schedule_str);
    let schedule = &Schedule::from_str(&schedule_str).expect("Failed to parse cron expression");
    loop {
        scheduler(schedule, max_pages).await;
    }
}

pub async fn next_schedule_time(schedule_str: &str) -> String {
    let now = chrono::Utc::now();
    let schedule = &Schedule::from_str(schedule_str).expect("Failed to parse cron expression");
    if let Some(next) = schedule.upcoming(chrono::Utc).next() {
        let duration_until_next = (next - now).to_std().expect("Failed to calculate duration");
        return format!("{:?}", next);
    }
    "No upcoming schedule".to_string()
}

async fn refresh_all_workloads(max_pages: usize) {
    log::info!("Refreshing all workloads");
    fetch_and_update_all_watched(max_pages)
        .await
        .unwrap_or_else(|e| log::error!("Error refreshing workloads: {}", e));
}
