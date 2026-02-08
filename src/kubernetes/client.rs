// kubernetes/client.rs
use crate::models::models::{UpdateStatus, Workload};
use futures::future::join_all;
use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{Api, ListParams},
    Client as KubeClient, Error as KubeError, ResourceExt,
};
use std::collections::BTreeMap;

pub struct Client {
    kube_client: KubeClient,
}

impl Client {
    pub async fn new() -> Result<Self, KubeError> {
        let kube_client = KubeClient::try_default().await?;
        Ok(Client { kube_client })
    }

    pub async fn list_pods(&self) -> Result<Vec<Pod>, KubeError> {
        let pods: Api<Pod> = Api::all(self.kube_client.clone());
        pods.list(&ListParams::default())
            .await
            .map(|pod_list| pod_list.items)
    }
}

async fn create_workload_from_pod(pod: Pod) -> Option<Workload> {
    let annotations = pod.metadata.annotations.as_ref()?;
    if annotations.get("slackwatch.enable") != Some(&"true".to_string()) {
        return None;
    }

    let namespace = pod.metadata.namespace.as_ref()?;
    let spec = pod.spec.as_ref()?;
    let container = spec.containers.first()?;
    let name = container.name.clone();
    let image = container.image.clone().unwrap_or_default();
    let image_parts: Vec<&str> = image.split(':').collect();
    let current_version = image_parts.get(1).unwrap_or(&"latest").to_string();

Some(Workload {
         name: name.clone(),
         namespace: namespace.clone(),
         image: image,
         current_version: current_version, // Simplified for demonstration
         latest_version: "1.0.0".to_string(), // Simplified for demonstration
         exclude_pattern: annotations.get("slackwatch.exclude").cloned(),
         include_pattern: annotations.get("slackwatch.include").cloned(),
         git_ops_repo: annotations.get("slackwatch.repo").cloned(),
         git_directory: annotations.get("slackwatch.directory").cloned(),
         update_available: UpdateStatus::NotAvailable, // Default value, adjust as needed
         last_scanned: chrono::Utc::now().to_rfc3339(),
         scan_exhausted: "False".to_string(),
     })
 }

pub async fn find_specific_workload(
    request_name: &str,
    request_namespace: &str,
) -> Result<Workload, KubeError> {
    let client = Client::new().await?;
    let pods = client.list_pods().await?;
    for pod in pods {
        if let Some(workload) = create_workload_from_pod(pod).await {
            if workload.name == request_name && workload.namespace == request_namespace {
                return Ok(workload);
            }
        }
    }
    Err(KubeError::Api(kube::error::ErrorResponse {
        code: 404,
        message: "Workload not found".to_string(),
        reason: "Not Found".to_string(),
        status: "Failure".to_string(),
    }))
}

pub async fn find_enabled_workloads() -> Result<Vec<Workload>, KubeError> {
    let client = Client::new().await?;
    let pods = client.list_pods().await?;

    // Map pods to a Vec of Futures
    let futures: Vec<_> = pods
        .into_iter()
        .map(|pod| create_workload_from_pod(pod.clone()))
        .collect();

    // Await all futures and filter out None values
    let workloads: Vec<Workload> = join_all(futures)
        .await
        .into_iter()
        .filter_map(|workload_option| workload_option)
        .collect();

    Ok(workloads)
}
