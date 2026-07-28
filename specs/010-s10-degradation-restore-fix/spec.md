# Feature Specification: S10 Degradation Restore Fix

**Feature Branch**: `010-s10-degradation-restore-fix`

**Created**: 2026-07-28

**Status**: Draft

**Input**: User description: "The S10 issue should be fixed." S10 (CRD
instances deleted → admission rejected) fails because the `restore_readiness`
function in the degradation scenario module returns before the Kubernetes
Service endpoints are repopulated, causing S10's probe to hit the S9
degradation (no endpoints) instead of the intended S10 degradation (capacity
data missing).

## User Scenarios & Testing *(mandatory)*

### User Story 1 — S10 Correctly Tests Capacity-Data-Missing (Priority: P1)

An operator runs `erw-verify` against a real cluster. The degradation suite runs
S9 (kill webhook pods), S10 (delete capacity CRD instances), and S11 (stale
capacity data) sequentially. Each scenario degrades the webhook, asserts the
expected fail-closed rejection, then restores health. S10 must observe a
capacity-data-missing rejection (the webhook detects the deleted CRD instances
and rejects with "capacity data unavailable"), NOT an unreachable rejection
(the Service has no endpoints because S9's restore is incomplete).

The root cause: `restore_readiness` in `degradation.rs` waits for pods to be
Ready and the Allocation ceiling to be non-zero, but it does NOT wait for the
Service's Endpoints to be populated. After S9 kills all pods, the Deployment
recreates them, they reach Ready, and the ceiling is repopulated — but the
Service's Endpoints controller has a propagation delay. S10's first probe
arrives before the Endpoints are ready, so the apiserver reports "no endpoints
available" (S9's failure mode) instead of forwarding the request to the webhook
(S10's failure mode).

**Why this priority**: S10 is the only scenario that tests the
CapacityDataMissing fail-closed path on real infrastructure. When it fails for
the wrong reason, the operator gets a false negative — the test reports failure
when the webhook's behaviour may actually be correct. This undermines trust in
the entire degradation suite.

**Independent Test**: run `erw-verify` against a clean cluster; S10 must pass
(reject with capacity-data-unavailable message) on the first run, without
requiring retries or manual intervention.

**Acceptance Scenarios**:

1. **Given** S9 has completed and `restore_readiness` has returned, **When** S10
deletes the capacity CRD instances and probes admission, **Then** the admission
request reaches the webhook (not blocked by missing endpoints) and the webhook
rejects with a capacity-data-unavailable message.
2. **Given** the degradation suite runs S9 → S10 → S11 sequentially, **When** each
scenario's restore phase completes, **Then** the next scenario's probes are
forwarded to a reachable webhook (the Service has ready endpoints).
3. **Given** S10 runs after a correct restore, **When** the capacity CRD instances
are deleted, **Then** the webhook rejects the next pod submission with a message
containing "capacity data unavailable" (the expected reason, not an endpoints
error).

---

### Edge Cases

- **Slow Endpoints controller**: on some clusters, the Endpoints controller may
take 10-20 seconds after pods are Ready to populate the Service Endpoints. The
restore must tolerate this propagation delay without timing out.
- **Zero Ready pods during restore**: if the Deployment has not yet recreated
any pods, the Endpoints will be empty. The restore must wait for pods first,
then for Endpoints.
- **Endpoints populated but stale**: the Endpoints may reference pods that are
Ready but the webhook process inside has not yet started serving (cold start).
The restore should confirm the webhook is actually reachable, not just that
Endpoints exist.
- **S10 deletes CRD instances faster than controller recreates them**: the test
must race the deletion against the controller's reconcile, probing repeatedly
until the webhook observes the missing data.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The `restore_readiness` function in the degradation module MUST
verify that the webhook Service has at least one ready endpoint before
returning, in addition to the existing checks (pods Ready + ceiling non-zero).
- **FR-002**: The readiness check MUST confirm the webhook is actually reachable
via the Service (not just that Endpoints exist), by making a request that
reaches the webhook and returns a response.
- **FR-003**: The restore timeout MUST accommodate Endpoints propagation delay
(up to 30 seconds on slower clusters) without prematurely declaring success.
- **FR-004**: S10's probe classification MUST distinguish between an unreachable
rejection (no endpoints — S9's failure mode) and a capacity-data-missing
rejection (S10's intended failure mode), reporting the latter as the expected
outcome and the former as unexpected.
- **FR-005**: The fix MUST NOT change the behaviour of S9 or S11 — S9 still tests
unreachable rejection, S11 still tests stale-data rejection.

### Key Entities

- **Service Endpoints Readiness**: a new readiness dimension alongside pod Ready
and ceiling non-zero. The Kubernetes Endpoints controller populates
`Endpoints`/`EndpointSlice` objects for a Service when its backing pods are
Ready and passing readiness probes. The restore must wait for this to happen.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Running `erw-verify` against a clean cluster, S10 passes on the
first run (no retries needed), reporting a capacity-data-unavailable rejection.
- **SC-002**: The restore between S9 and S10 completes within the existing
restore timeout (60 seconds) on a typical cluster, without extending the
overall test duration significantly.
- **SC-003**: The fix does not introduce flakiness into S9 or S11 — both continue
to pass reliably.

## Assumptions

- The cluster's Endpoints controller is functional (not broken or deliberately
delayed).
- The webhook Deployment's readiness probe (`/healthz` on the metrics port) is
configured correctly in the existing manifests.
- The existing `RESTORE_TIMEOUT` (60 seconds) is sufficient for the combined
pod-readiness + endpoints-propagation + ceiling-repopulation sequence on a
typical cluster. If empirical testing shows it is not, the timeout is a
plan-phase tuning decision.
- The fix is confined to the `erw-verify` binary's degradation scenario module
(`src/bin/erw-verify/scenarios/degradation.rs`); the webhook's production code is
not changed.
