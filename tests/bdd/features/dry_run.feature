Feature: Dry-run enforcement mode
  As a cluster operator
  I want to install the webhook in an audit (dry-run) mode
  So that over-budget pods are admitted with a warning instead of rejected,
  while fail-closed paths still protect the cluster.

  In dry-run mode the webhook evaluates every admission normally but converts an
  over-budget denial into an admission carrying the would-be rejection as a
  warning. Capacity data that is stale or missing still rejects regardless of
  mode (Constitution Principle I). Switching the mode is a spec patch that takes
  effect on the next decision (FR-002).

  Background:
    Given the cluster has 100 CPU and 200 GiB allocatable
    And the budget is 80 percent
    And the current allocation is 70 CPU and 110 GiB

  # Spec-004 US1 acceptance scenario 1
  Scenario: Dry-run admits an over-budget pod with a warning
    Given the enforcement mode is "dry-run"
    When a pod requesting 15 CPU and 10 GiB is submitted
    Then the pod is admitted
    And the admission warning contains "Budget violations (dry-run):"
    And the admission warning contains "CPU budget exceeded"
    And the admission warning contains "projected 85000m"

  # Spec-004 US2: fail-closed paths reject in dry-run mode too (FR-006)
  Scenario: Dry-run rejects on stale capacity data
    Given the enforcement mode is "dry-run"
    And the allocation was last refreshed 60 seconds ago
    When a pod requesting 15 CPU and 10 GiB is submitted
    Then the pod is rejected
    And the rejection message contains "capacity data unavailable"
    And the admission carries no warning

  # Spec-004 US1: enforce mode is unchanged
  Scenario: Enforce mode rejects an over-budget pod
    Given the enforcement mode is "enforce"
    When a pod requesting 15 CPU and 10 GiB is submitted
    Then the pod is rejected
    And the rejection message contains "CPU budget exceeded"
    And the rejection message contains "projected 85000m"
    And the admission carries no warning
