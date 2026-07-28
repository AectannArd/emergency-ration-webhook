# Data Model: S10 Degradation Restore Fix

## §1 — Restore readiness dimensions

The `restore_readiness` function currently checks two dimensions. The fix adds
a third.

| Dimension | Check | Existing? |
|-----------|-------|-----------|
| Pods Ready | At least one webhook pod is `Running` with all containers ready | Yes |
| Ceiling non-zero | `Allocation.status.ceilingCpuMilli > 0` | Yes |
| Service Endpoints populated | `Endpoints.capacity-admission-webhook` has ≥1 address in `subsets[].addresses[]` | **NEW** |

## §2 — Endpoints readiness check

```rust
/// Check that the webhook Service has at least one ready endpoint.
async fn endpoints_ready(client: &Client) -> bool {
    let endpoints: Api<Endpoints> = Api::namespaced(client.clone(), NAMESPACE);
    match endpoints.get(SERVICE_NAME).await {
        Ok(ep) => ep.subsets.as_ref()
            .map(|subsets| subsets.iter()
                .flat_map(|s| s.addresses.as_deref().unwrap_or(&[]))
                .any(|_| true))
            .unwrap_or(false),
        Err(_) => false,
    }
}
```

The existing `wait_for_readiness` loop gains this as a third condition in its
polling predicate. The loop already polls every 2 seconds with a 60-second
timeout — sufficient for Endpoints propagation.

## §3 — No data model changes

No new CRDs, no new status fields, no new Kubernetes objects. This is purely a
readiness-checking enhancement inside the verify binary.
