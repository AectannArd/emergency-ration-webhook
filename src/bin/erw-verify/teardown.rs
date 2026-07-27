//! Teardown (spec-005, research R12): delete everything the setup installed, in
//! reverse dependency order. Each deletion waits for the object to be fully gone
//! (poll `.get()` until 404) before proceeding, because CRDs and namespaces
//! delete asynchronously via finalizers. Partial failures are collected and
//! surfaced as a single error (→ exit code 3) so a partly-cleaned cluster is
//! never reported as success.

use std::time::Duration;

use k8s_openapi::api::admissionregistration::v1::ValidatingWebhookConfiguration;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{Namespace, Secret, Service, ServiceAccount};
use k8s_openapi::api::rbac::v1::{ClusterRole, ClusterRoleBinding};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::Client;
use kube::api::{Api, DeleteParams};
use tracing::warn;

use capacity_admission_webhook::crd::{
    Allocation, CLUSTER_ALLOCATION_NAME, CLUSTER_CAPACITY_NAME, ClusterCapacity,
};

use crate::error::{Result, err};

const NAMESPACE: &str = "capacity-admission";
const TLS_SECRET_NAME: &str = "capacity-admission-webhook-tls";
const DEPLOYMENT_NAME: &str = "capacity-admission-webhook";
const SERVICE_NAME: &str = "capacity-admission-webhook";
const SA_NAME: &str = "capacity-admission-webhook";
const WEBHOOK_CONFIG_NAME: &str = "capacity-admission.emergency-ration.dev";
const CRD_ALLOCATION: &str = "allocations.emergency-ration.dev";
const CRD_CAPACITY: &str = "clustercapacities.emergency-ration.dev";

/// Per-object deletion timeout. CRD and namespace finalizers can take a while.
const DELETE_TIMEOUT: Duration = Duration::from_secs(180);

/// Delete everything the verify tool installed, in reverse dependency order.
///
/// Returns `Ok(())` only if every object was removed; otherwise a single error
/// joining the per-object failures (the orchestrator maps this to exit code 3).
pub async fn teardown(client: &Client) -> Result<()> {
    let mut errors: Vec<String> = Vec::new();

    // 1. Stop the apiserver forwarding to the webhook.
    try_delete(
        &Api::<ValidatingWebhookConfiguration>::all(client.clone()),
        WEBHOOK_CONFIG_NAME,
        &mut errors,
    )
    .await;
    // 2. Stop the webhook pods (kills the controllers before deleting CRD data).
    try_delete(
        &Api::<Deployment>::namespaced(client.clone(), NAMESPACE),
        DEPLOYMENT_NAME,
        &mut errors,
    )
    .await;
    // 3. Service.
    try_delete(
        &Api::<Service>::namespaced(client.clone(), NAMESPACE),
        SERVICE_NAME,
        &mut errors,
    )
    .await;
    // 4. TLS Secret.
    try_delete(
        &Api::<Secret>::namespaced(client.clone(), NAMESPACE),
        TLS_SECRET_NAME,
        &mut errors,
    )
    .await;
    // 5. RBAC (binding, role, account).
    try_delete(
        &Api::<ClusterRoleBinding>::all(client.clone()),
        SA_NAME,
        &mut errors,
    )
    .await;
    try_delete(
        &Api::<ClusterRole>::all(client.clone()),
        SA_NAME,
        &mut errors,
    )
    .await;
    try_delete(
        &Api::<ServiceAccount>::namespaced(client.clone(), NAMESPACE),
        SA_NAME,
        &mut errors,
    )
    .await;
    // 6. CRD instances (must precede the CRDs themselves).
    try_delete(
        &Api::<Allocation>::all(client.clone()),
        CLUSTER_ALLOCATION_NAME,
        &mut errors,
    )
    .await;
    try_delete(
        &Api::<ClusterCapacity>::all(client.clone()),
        CLUSTER_CAPACITY_NAME,
        &mut errors,
    )
    .await;
    // 7. CRDs.
    try_delete(
        &Api::<CustomResourceDefinition>::all(client.clone()),
        CRD_ALLOCATION,
        &mut errors,
    )
    .await;
    try_delete(
        &Api::<CustomResourceDefinition>::all(client.clone()),
        CRD_CAPACITY,
        &mut errors,
    )
    .await;
    // 8. Namespace (last — cascade-cleans any straggler).
    try_delete(
        &Api::<Namespace>::all(client.clone()),
        NAMESPACE,
        &mut errors,
    )
    .await;

    if errors.is_empty() {
        Ok(())
    } else {
        Err(err(format!(
            "teardown completed with {} error(s): {}",
            errors.len(),
            errors.join("; ")
        )))
    }
}

/// Delete `name` via `api`, wait until it is gone (404), and push any failure
/// into `errors` (never aborts the whole teardown on one object).
async fn try_delete<K>(api: &Api<K>, name: &str, errors: &mut Vec<String>)
where
    K: Clone + serde::de::DeserializeOwned + std::fmt::Debug,
{
    if let Err(e) = delete_and_wait(api, name).await {
        warn!(%name, error = %e, "teardown: failed to delete object");
        errors.push(format!("{name}: {e}"));
    }
}

/// Delete an object and poll until the apiserver reports it gone (404). A 404 on
/// the initial delete is success (already absent).
async fn delete_and_wait<K>(api: &Api<K>, name: &str) -> Result<()>
where
    K: Clone + serde::de::DeserializeOwned + std::fmt::Debug,
{
    match api.delete(name, &DeleteParams::default()).await {
        Ok(_) => {}
        Err(kube::Error::Api(status)) if status.code == 404 => return Ok(()),
        Err(e) => return Err(e.into()),
    }
    let deadline = tokio::time::Instant::now() + DELETE_TIMEOUT;
    loop {
        match api.get(name).await {
            Ok(_) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(err(format!(
                        "timed out waiting for {name} to be fully deleted"
                    )));
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(kube::Error::Api(status)) if status.code == 404 => return Ok(()),
            Err(e) => return Err(e.into()),
        }
    }
}
