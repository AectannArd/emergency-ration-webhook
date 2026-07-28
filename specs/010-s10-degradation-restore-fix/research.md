# Research: S10 Degradation Restore Fix

## R1: Root cause analysis

**Symptom**: S10 fails with "rejected, but not for the expected reason: no
endpoints available for service" instead of the expected "capacity data
unavailable" rejection.

**Root cause**: `restore_readiness` in `degradation.rs:310` calls
`wait_for_readiness` which checks two conditions:
1. At least one webhook pod is `Running` with ready containers (`pod_ready`)
2. The Allocation `ceilingCpuMilli` is non-zero

It does NOT check that the Service's Endpoints are populated. After S9 kills
all webhook pods, the Deployment recreates them. The pods reach Ready and the
ceiling is repopulated quickly (~5-10s). But the Kubernetes Endpoints
controller — which watches pods and populates `Endpoints`/`EndpointSlice`
objects for the Service — has a propagation delay (typically 2-15 seconds).

S10's first probe arrives before the Endpoints are populated. The apiserver
tries to forward the admission request to the webhook Service, finds no
endpoints, and returns "no endpoints available" — which is S9's failure mode
(unreachable), not S10's (capacity data missing).

## R2: Fix approach — Endpoints readiness check

**Decision**: add a Service Endpoints readiness check to `restore_readiness`.

After the existing checks (pods Ready + ceiling non-zero), poll the
Service's Endpoints until at least one address is available. This ensures the
apiserver can forward admission requests to the webhook before S10 begins its
degradation.

**Implementation**: read the `Endpoints` object for the webhook Service:
```rust
let endpoints: Api<Endpoints> = Api::namespaced(client.clone(), NAMESPACE);
let ep = endpoints.get("capacity-admission-webhook").await?;
// Check ep.subsets[].addresses[] is non-empty
```

**Alternative considered**: make an HTTP request to the webhook's `/healthz`
via the Service DNS. This would test full reachability but requires a network
call from the operator's machine into the cluster (which may not be reachable
from outside). Reading Endpoints is an apiserver query, which the tool already
does. Endpoints is the right signal.

## R3: Endpoints propagation timing

Empirical data from the test run: after S9 kills pods, the Deployment recreates
them in ~5 seconds. Pods reach Ready ~5-10 seconds after creation. The
Endpoints controller populates addresses 2-15 seconds after pods become Ready.

Total restore time: ~15-35 seconds. The existing `RESTORE_TIMEOUT` of 60
seconds provides ample headroom.

## R4: Classification robustness

The `expect_capacity_unavailable` classifier checks for
`detail.contains("capacity data unavailable")`. If S10 still races and hits
the no-endpoints error, the classifier correctly identifies it as unexpected
(`ProbeKind::Unexpected`). The fix targets the root cause (restore returning
too early) rather than the symptom (classification).

However, as defence-in-depth, the error message for an unexpected unreachable
rejection during S10 should hint at the restore timing: "rejected with
unreachable error during S10 — the Service may not have had ready endpoints
yet". This helps the operator diagnose future races.
