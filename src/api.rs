use warp::{Filter, Rejection, Reply};
use warp::filters::cors::cors;
use warp::http::Method;
use serde_json::json;
use std::collections::HashMap;
use crate::models::models::Workload;
use crate::config::Settings;
use crate::services::workloads::{fetch_and_update_all_watched, update_single_workload};
use crate::gitops::gitops::run_git_operations;
use crate::services::scheduler::next_schedule_time;
use crate::database::client::{return_all_workloads, return_workload};
use crate::notifications::ntfy::{notify_commit, schedule_rescan};

pub async fn start_api_server() {
    // CORS configuration
    let cors = cors()
        .allow_any_origin()
        .allow_methods(&[Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(vec!["Content-Type"]);

    // API routes
    let api = warp::path("api");

    // GET /api/workloads - Get all workloads
    let get_workloads = api
        .and(warp::path("workloads"))
        .and(warp::get())
        .and_then(handle_get_workloads);

    // POST /api/workloads/update - Update a workload
    let update_workload = api
        .and(warp::path("workloads"))
        .and(warp::path("update"))
        .and(warp::post())
        .and(warp::body::json())
        .and_then(handle_update_workload);

    // POST /api/workloads/upgrade - Upgrade a workload
    let upgrade_workload = api
        .and(warp::path("workloads"))
        .and(warp::path("upgrade"))
        .and(warp::post())
        .and(warp::body::json())
        .and_then(handle_upgrade_workload);

    // POST /api/workloads/refresh-all - Refresh all workloads
    let refresh_all = api
        .and(warp::path("workloads"))
        .and(warp::path("refresh-all"))
        .and(warp::post())
        .and_then(handle_refresh_all);

    // GET /api/settings - Get settings
    let get_settings = api
        .and(warp::path("settings"))
        .and(warp::get())
        .and_then(handle_get_settings);

    // GET /api/settings/next-schedule-time - Get next schedule time
    let get_next_schedule = api
        .and(warp::path("settings"))
        .and(warp::path("next-schedule-time"))
        .and(warp::get())
        .and_then(handle_get_next_schedule);

    // POST /api/ntfy/callback - Handle ntfy action callbacks
    let ntfy_callback = api
        .and(warp::path("ntfy"))
        .and(warp::path("callback"))
        .and(warp::post())
        .and(warp::query::<HashMap<String, String>>())
        .and_then(handle_ntfy_callback);

    // Serve static files from the frontend/dist directory
    let static_files = warp::fs::dir("frontend/dist");

    // Fallback route for SPA - serve index.html for any other route
    let spa_fallback = warp::any()
        .and(warp::fs::file("frontend/dist/index.html"));

    // Combine all routes
    let routes = get_workloads
        .or(update_workload)
        .or(upgrade_workload)
        .or(refresh_all)
        .or(get_settings)
        .or(get_next_schedule)
        .or(ntfy_callback)
        .or(static_files)
        .or(spa_fallback)
        .with(cors);

    // Start the server
    log::info!("Starting API server on 0.0.0.0:8080");
    warp::serve(routes)
        .run(([0, 0, 0, 0], 8080))
        .await;
}

async fn handle_get_workloads() -> Result<impl Reply, Rejection> {
    match return_all_workloads() {
        Ok(workloads) => Ok(warp::reply::json(&workloads)),
        Err(e) => {
            log::error!("Failed to get workloads: {}", e);
            let error = json!({ "error": format!("Failed to get workloads: {}", e) });
            Ok(warp::reply::json(&error))
        }
    }
}

async fn handle_update_workload(workload: Workload) -> Result<impl Reply, Rejection> {
    match update_single_workload(workload).await {
        Ok(_) => Ok(warp::reply::json(&json!({ "status": "success" }))),
        Err(e) => {
            log::error!("Failed to update workload: {}", e);
            let error = json!({ "error": format!("Failed to update workload: {}", e) });
            Ok(warp::reply::json(&error))
        }
    }
}

async fn handle_upgrade_workload(workload: Workload) -> Result<impl Reply, Rejection> {
    match run_git_operations(workload).await {
        Ok(_) => Ok(warp::reply::json(&json!({ "status": "success" }))),
        Err(e) => {
            log::error!("Failed to upgrade workload: {}", e);
            let error = json!({ "error": format!("Failed to upgrade workload: {}", e) });
            Ok(warp::reply::json(&error))
        }
    }
}

async fn handle_refresh_all() -> Result<impl Reply, Rejection> {
    match fetch_and_update_all_watched().await {
        Ok(_) => Ok(warp::reply::json(&json!({ "status": "success" }))),
        Err(e) => {
            log::error!("Failed to refresh all workloads: {}", e);
            let error = json!({ "error": format!("Failed to refresh all workloads: {}", e) });
            Ok(warp::reply::json(&error))
        }
    }
}

async fn handle_get_settings() -> Result<impl Reply, Rejection> {
    match Settings::new() {
        Ok(settings) => Ok(warp::reply::json(&settings)),
        Err(e) => {
            log::error!("Failed to get settings: {}", e);
            let error = json!({ "error": format!("Failed to get settings: {}", e) });
            Ok(warp::reply::json(&error))
        }
    }
}

async fn handle_get_next_schedule() -> Result<impl Reply, Rejection> {
    match Settings::new() {
        Ok(settings) => {
            let schedule_str = &settings.system.schedule;
            let next_schedule = next_schedule_time(&schedule_str).await;
            // Ensure we're returning a string, not an object
            Ok(warp::reply::json(&next_schedule))
        },
        Err(e) => {
            log::error!("Failed to get settings for next schedule: {}", e);
            let error = json!({ "error": format!("Failed to get settings for next schedule: {}", e) });
            Ok(warp::reply::json(&error))
        }
    }
}

async fn handle_ntfy_callback(
    query: HashMap<String, String>,
) -> Result<impl Reply, Rejection> {
    let action = query.get("action").cloned();
    let namespace = query.get("namespace").cloned().unwrap_or_else(|| "default".to_string());
    let provided_token = query.get("token").cloned();

    // Validate token if configured
    if let Ok(settings) = Settings::new() {
        if let Some(ref notifications) = settings.notifications {
            if let Some(ref ntfy) = notifications.ntfy {
                if let Some(ref expected_token) = ntfy.callback_token {
                    if provided_token.as_deref() != Some(expected_token.as_str()) {
                        log::warn!("Unauthorized upgrade attempt: invalid token");
                        return Ok(warp::reply::with_status(
                            "Unauthorized".to_string(),
                            warp::http::StatusCode::UNAUTHORIZED,
                        ));
                    }
                }
            }
        }
    }

    match action {
        Some(name) => {
            match return_workload(name.clone(), namespace.clone()) {
                Ok(workload) => {
                    let wl = workload.clone();
                    match run_git_operations(wl.clone()).await {
                        Ok(_) => {
                            let _ = notify_commit(&wl).await;

                            if let Ok(settings) = Settings::new() {
                                if let Some(ref notifications) = settings.notifications {
                                    if let Some(ref ntfy) = notifications.ntfy {
                                        schedule_rescan(wl.clone(), &ntfy.auto_rescan_delay).await;
                                    }
                                }
                            }

                            Ok(warp::reply::with_status(
                                format!("Upgrade initiated for {}", name),
                                warp::http::StatusCode::OK,
                            ))
                        }
                        Err(e) => {
                            log::error!("GitOps failed for {}: {}", name, e);
                            Ok(warp::reply::with_status(
                                format!("Upgrade failed for {}: {}", name, e),
                                warp::http::StatusCode::OK,
                            ))
                        }
                    }
                }
                Err(_) => {
                    log::error!("Workload {} not found in DB", name);
                    Ok(warp::reply::with_status(
                        format!("Workload {} not found", name),
                        warp::http::StatusCode::OK,
                    ))
                }
            }
        }
        None => {
            Ok(warp::reply::with_status(
                "No action parameter provided".to_string(),
                warp::http::StatusCode::OK,
            ))
        }
    }
}
