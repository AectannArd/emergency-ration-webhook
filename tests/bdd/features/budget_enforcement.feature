Feature: Budget enforcement
  As a cluster operator
  I want workloads that exceed the capacity budget to be rejected
  So that the cluster is protected from overcommit.

  The budget is a hard ceiling on the sum of declared pod resource requests,
  expressed as a percentage of total allocatable CPU and RAM. A pod that fits is
  admitted; one that would push allocation over the ceiling is rejected with the
  violated resource and the budget figures.

  Background:
    Given the current allocation is 70000m CPU and 110 GiB memory
    And the budget ceiling is 80000m CPU and 160 GiB memory

  # Spec US1 acceptance scenario 1
  Scenario: A pod that fits within the budget is admitted
    When a pod requesting 5000m CPU and 40 GiB memory is submitted
    Then the pod is admitted

  # Spec US1 acceptance scenario 2
  Scenario: A pod that exceeds the CPU budget is rejected with figures
    When a pod requesting 15000m CPU and 10 GiB memory is submitted
    Then the pod is rejected
    And the rejection message contains "allocated 70000m"
    And the rejection message contains "requested 15000m"
    And the rejection message contains "projected 85000m"
    And the rejection message contains "ceiling 80000m"

  # Spec US1 acceptance scenario 3 — the ceiling is inclusive
  Scenario: A pod landing exactly at the ceiling is admitted
    Given the current allocation is 75000m CPU and 0 GiB memory
    And the budget ceiling is 80000m CPU and 0 GiB memory
    When a pod requesting 5000m CPU and 0 GiB memory is submitted
    Then the pod is admitted

  # Spec US1 acceptance scenario 4
  Scenario: A pod requesting nothing is admitted
    When a pod requesting 0m CPU and 0 GiB memory is submitted
    Then the pod is admitted

  # Spec US1 acceptance scenario 5 — updates evaluate the delta
  Scenario: An update is evaluated as the delta against the budget
    Given an existing pod consuming 10000m CPU
    When the pod is updated to request 20000m CPU
    Then the pod is admitted

  Scenario: An update whose delta exceeds the budget is rejected
    Given an existing pod consuming 10000m CPU
    When the pod is updated to request 30000m CPU
    Then the pod is rejected
    And the rejection message contains "projected 90000m"

  # Spec-012 US1 AC1 — per-resource asymmetric budgets: CPU admits, memory denies.
  # The ceilings come from the resolved per-resource budgets (95% CPU, 30% memory),
  # computed exactly as the controller would (resolve_effective_budgets →
  # ceiling_per_resource). A CPU-heavy pod fits the 95% CPU ceiling but blows the
  # 30% memory ceiling → rejected on memory ONLY (FR-011).
  Scenario: Per-resource asymmetric budgets — CPU admits, memory denies
    Given the cluster has 100000m CPU and 200 GiB allocatable
    And the budget is 80% with cpuBudgetPercent 95 and memoryBudgetPercent 30
    And the current allocation is 0m CPU and 0 GiB memory
    When a pod requesting 90000m CPU and 150 GiB memory is submitted
    Then the pod is rejected
    And the rejection message contains "memory budget exceeded"
    And the rejection message does not contain "CPU budget exceeded"

  # Spec-012 US1 AC2 — swapped overrides: CPU denies, memory admits.
  # Same pod, but 30% CPU / 95% memory ceilings → rejected on CPU ONLY.
  Scenario: Per-resource asymmetric budgets swapped — CPU denies, memory admits
    Given the cluster has 100000m CPU and 200 GiB allocatable
    And the budget is 80% with cpuBudgetPercent 30 and memoryBudgetPercent 95
    And the current allocation is 0m CPU and 0 GiB memory
    When a pod requesting 90000m CPU and 150 GiB memory is submitted
    Then the pod is rejected
    And the rejection message contains "CPU budget exceeded"
    And the rejection message does not contain "memory budget exceeded"
