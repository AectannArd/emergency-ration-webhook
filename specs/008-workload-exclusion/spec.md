# Feature Specification: Workload Exclusion Policy

**Feature Branch**: `008-workload-exclusion`

**Created**: 2026-07-27

**Status**: Draft

**Input**: The webhook's own namespace is currently excluded via a static
`namespaceSelector` in the `ValidatingWebhookConfiguration` manifest, plus a
`--namespace` / `NAMESPACE` CLI/ENV var used only for logging. This is
inflexible — operators cannot exclude other namespaces or workloads by
priority class without editing YAML manifests and restarting the webhook. This
feature moves the exclusion policy into a CRD so it is dynamically
configurable, and introduces two selection types: by namespace list and by
priority class list.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Exclude Workloads by Namespace List (Priority: P1)

As a cluster operator, I want to configure a list of namespaces on the
Allocation CRD whose pods are exempt from capacity admission, so that
critical system workloads or CI/CD namespaces are never blocked by the
capacity budget — without editing the `ValidatingWebhookConfiguration`
manifest.

**Why this priority**: this is the direct CRD migration of the existing static
namespace exclusion. It replaces the hardcoded webhook-namespace + system-
namespace list with a dynamic, operator-controlled field. Without it, every
namespace-exclusion change requires a manifest edit and webhook restart.

**Independent Test**: configure an excluded namespace on the Allocation CRD,
submit a pod into that namespace that would exceed the capacity budget, and
verify it is admitted (exempt) rather than denied.

**Acceptance Scenarios**:

1. **Given** a cluster where the capacity budget is at 100% (no remaining
   headroom), **When** the operator adds namespace `monitoring` to the
   exclusion list on the Allocation CRD, **Then** a new pod created in
   `monitoring` is admitted immediately — no capacity check is performed
   against it.
2. **Given** namespace `monitoring` is in the exclusion list, **When** a pod
   is created in a non-excluded namespace `app-team-a`, **Then** the pod IS
   subject to the capacity budget check and is denied if over budget.
3. **Given** two namespaces are excluded (`monitoring`, `ci-cd`), **When** the
   operator removes `ci-cd` from the list, **Then** new pods in `ci-cd` are
   again subject to capacity admission (a subsequent over-budget pod in
   `ci-cd` is denied).
4. **Given** no exclusion list is configured (field absent), **When** any pod
   is created in any namespace, **Then** it is subject to capacity admission
   as before (backward-compatible default).

---

### User Story 2 - Exclude Workloads by Priority Class (Priority: P2)

As a cluster operator, I want to configure a list of Kubernetes priority
classes whose pods are exempt from capacity admission, so that high-priority
workloads (e.g. `system-node-critical`, `system-cluster-critical`, or custom
critical-app priority classes) are never blocked by the capacity budget —
regardless of which namespace they run in.

**Why this priority**: priority class is orthogonal to namespace — it cuts
across namespaces and cannot be expressed by a `namespaceSelector` on the
webhook config. This is the second selection type the feature introduces, and
the one that has no workaround today (namespace exclusion at least had a
static manifest mechanism).

**Independent Test**: configure an excluded priority class on the Allocation
CRD, submit a pod with that priority class that would exceed the capacity
budget, and verify it is admitted (exempt).

**Acceptance Scenarios**:

1. **Given** a cluster at capacity budget exhaustion, **When** the operator
   adds priority class `system-node-critical` to the exclusion list, **Then**
   a pod with `priorityClassName: system-node-critical` is admitted even
   though the budget is full.
2. **Given** priority class `system-node-critical` is excluded, **When** a pod
   WITHOUT a priority class (or with a different, non-excluded class) is
   submitted in the same namespace, **Then** that pod IS subject to capacity
   admission.
3. **Given** priority class `gold` is excluded, **When** the operator removes
   `gold` from the list, **Then** subsequent pods with `priorityClassName:
   gold` are again subject to capacity admission.
4. **Given** a pod has no `priorityClassName` set (empty/absent), **When** it
   is evaluated against an exclusion list containing `system-node-critical`,
   **Then** the pod is NOT excluded — only an exact match on the configured
   priority class value exempts a pod.

---

### User Story 3 - Combined Namespace + Priority Class Exclusion (Priority: P3)

As a cluster operator, I want to configure BOTH namespace and priority class
exclusion lists simultaneously, so that a pod is exempt if it matches EITHER
criterion (OR semantics). This lets me express "exclude everything in the
`kube-system` namespace OR anything with `system-node-critical` priority,
anywhere in the cluster."

**Why this priority**: validates the interaction of the two selection types
and the OR semantics. This is the integration story — each type works
independently (US1, US2), and together they OR.

**Independent Test**: configure both lists, submit pods that match only the
namespace rule, only the priority rule, and neither rule, and verify the
correct exemption/subject-to-admission outcome for each.

**Acceptance Scenarios**:

1. **Given** namespace `kube-system` and priority class `system-node-critical`
   are both in their respective exclusion lists, **When** a pod with
   `priorityClassName: system-node-critical` is submitted in namespace
   `app-team-a` (not in the namespace list), **Then** the pod IS exempt
   (matched by priority class).
2. **Given** the same configuration, **When** a pod with no priority class is
   submitted in namespace `kube-system`, **Then** the pod IS exempt (matched
   by namespace).

3. **Given** the same configuration, **When** a pod with no priority class is
   submitted in namespace `app-team-a`, **Then** the pod IS subject to
   capacity admission (matched neither).
4. **Given** both lists are configured and a pod matches BOTH criteria
   (namespace `kube-system` AND `priorityClassName: system-node-critical`),
   **When** it is evaluated, **Then** it is exempt (counted once, not
   double-counted — exemption is boolean).

---

### Edge Cases

- **Pod with empty-string priorityClassName**: the API server treats an absent
  `priorityClassName` and an empty-string `priorityClassName` identically
  (no priority class). Both MUST NOT match any configured exclusion entry.
- **Exclusion list contains duplicate entries**: e.g. `["monitoring",
  "monitoring"]`. The webhook MUST treat this as a set — duplicates do not
  cause errors or double-evaluation.
- **Exclusion list is empty (`[]`)**: MUST behave identically to the field
  being absent (no exclusions). An empty list is not an error.
- **Excluded pod still appears in allocation accounting**: exclusion affects
  the admission decision ONLY. Excluded pods are still counted in the
  Allocation CRD's `status.allocated*` figures by the Allocation Controller
  — they consume real cluster resources. The exclusion means the webhook
  does not GATE them, not that they are invisible to accounting. (This
  preserves Principle II: the budget is a hard budget for non-excluded
  workloads, but excluded workloads can exceed it by design — the operator
  accepted that risk by listing them.)
- **Webhook's own namespace bootstrap**: the webhook MUST continue to exempt
  its own namespace so it does not gate itself during startup (the original
  problem the static `namespaceSelector` solved). With CRD-based exclusion,
  the webhook's own namespace MUST be in the exclusion list OR handled by a
  fallback. If the Allocation CRD is not yet created (cold start), the
  webhook MUST still exempt its own namespace to avoid a deadlock.
- **CRD updated mid-flight**: the webhook reads the exclusion config from the
  cached Allocation reflector, so a CRD update takes effect on the next
  reflector sync (sub-second). No restart is needed.
- **Non-existent priority class name in the exclusion list**: the operator
  types a priority class that doesn't exist in the cluster. No error — pods
  simply never match it. The list is a string match, not a reference to a
  real PriorityClass resource.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The Allocation CRD spec MUST accept an optional
  `excludedNamespaces` field — a list of namespace name strings. Pods whose
  namespace matches any entry are exempt from capacity admission.
- **FR-002**: The Allocation CRD spec MUST accept an optional
  `excludedPriorityClasses` field — a list of priority class name strings.
  Pods whose `spec.priorityClassName` matches any entry are exempt from
  capacity admission.
- **FR-003**: A pod MUST be exempt if it matches EITHER exclusion list (OR
  semantics). Matching both lists does not change the outcome (exemption is
  boolean).
- **FR-004**: When both fields are absent or empty, the webhook MUST admit
  no workloads by exclusion — every pod is subject to the capacity budget
  check as before. This is the backward-compatible default.
- **FR-005**: Exemption MUST mean the webhook returns `allowed: true` for the
  pod WITHOUT performing the budget check. The pod is still counted in
  allocation figures by the Allocation Controller (exclusion affects the
  admission gate only, not accounting).
- **FR-006**: The webhook MUST read the exclusion lists from the cached
  Allocation CRD reflector (same source as `budget_percent` and
  `enforcement_mode`). No additional API server calls on the admission hot
  path.
- **FR-007**: The webhook MUST always exempt its own namespace (from
  `--namespace` / `NAMESPACE` config), even if the Allocation CRD is absent
  or the exclusion fields are not configured. This prevents the self-gating
  bootstrap deadlock.
- **FR-008**: When the webhook exempts a pod by exclusion policy, it MUST
  emit a structured log entry and increment a Prometheus counter so operators
  can observe how many pods bypass the budget and why (Principle IV).
- **FR-009**: The `namespaceSelector` in `deploy/webhook-config.yaml` MUST be
  updated to remove the hardcoded namespace exclusion list (now in the CRD)
  but MUST keep a minimal selector for the webhook's own namespace as a
  defence-in-depth — the webhook must never gate its own namespace, even
  during cold start before the CRD cache is populated.
- **FR-010**: The deprecated `--namespace` / `NAMESPACE` config MUST remain
  for the webhook's own namespace (FR-007). It is NOT removed — it is the
  bootstrap fallback that guarantees the webhook never self-gates.

### Key Entities *(include if feature involves data)*

- **Allocation CRD spec** — gains two new optional fields:
  `excludedNamespaces: []string` and `excludedPriorityClasses: []string`.
  Both default to absent/empty. They live alongside `budget_percent` and
  `enforcement_mode` because exclusion is an admission-policy concern, and
  the Allocation CRD is the admission policy singleton (Constitution
  Principle V: the webhook reads the Allocation spec for its policy).
- **ValidatingWebhookConfiguration** — the static `namespaceSelector` is
  simplified: it keeps only the webhook's own namespace as a safety net (the
  apiserver filters the webhook's own namespace before the request reaches
  the webhook), but the system-namespace exclusion moves to the CRD where it
  is dynamically configurable.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can add or remove a namespace from the admission
  exclusion set by patching the Allocation CRD, and the change takes effect
  for the next admission request in that namespace — no manifest edit, no
  webhook restart, no redeployment.
- **SC-002**: An operator can exclude workloads by priority class — a
  capability that was impossible before (priority class is not a namespace
  attribute and cannot be matched by `namespaceSelector`).
- **SC-003**: Every exempted admission is observable: the structured log
  carries the namespace and/or priority class that triggered the exemption,
  and a Prometheus counter tracks total exemptions by selection type.
- **SC-004**: Existing clusters upgrading to this feature experience no
  behaviour change when the new fields are absent — all pods continue to be
  subject to the capacity budget exactly as before.
- **SC-005**: The webhook never gates its own namespace under any
  configuration, including cold start with no CRD cached.

## Assumptions

- The Allocation CRD is the correct home for the exclusion fields because it
  is already the admission-policy singleton (it holds `budget_percent` and
  `enforcement_mode`). The ClusterCapacity CRD governs supply-side node
  counting and is not the right place for admission-policy fields.
- Priority class exclusion is a string match on `pod.spec.priorityClassName`
  against the configured list. The webhook does NOT resolve or validate
  against actual `PriorityClass` resources — it matches the string value the
  pod carries. (This avoids an extra API call and a cluster-scoped resource
  watch.)
- The webhook's own namespace remains configured via the existing
  `--namespace` / `NAMESPACE` flag/ENV. This value is used for the bootstrap
  self-exemption (FR-007) and is NOT deprecated by this feature.
- The `ValidatingWebhookConfiguration` `namespaceSelector` for the webhook's
  own namespace is kept as defence-in-depth (FR-009). The apiserver filters
  the webhook's own namespace before the request reaches the webhook, so
  even a misconfigured or absent CRD cannot cause self-gating during cold
  start.
- Excluded workloads are still counted in allocation accounting. Exclusion
  is an admission-gate bypass only, not an accounting exclusion. This is a
  deliberate design choice: the operator accepts that excluded workloads can
  exceed the budget, but their resource consumption is still visible in the
  Allocation status for observability.
