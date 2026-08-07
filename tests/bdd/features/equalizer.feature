Feature: Multi-cluster capacity equalizer (spec-013)
  The equalizer converges fleet-wide capacity budgets toward the configured
  target by tuning per-cluster Allocation overrides. These scenarios drive the
  real reconcile loop against mocked target apiservers.

  # US1 / quickstart V1.4
  Scenario: All clusters within target — budgets set to target
    Given 3 target clusters with CPU utilization 65%, 55%, 45%
    And the EqualizerConfig has cpuTargetBudgetPercent 80
    When the equalizer reconciles the fleet
    Then each cluster receives cpuBudgetPercent 80
    And the fleet condition is Healthy
