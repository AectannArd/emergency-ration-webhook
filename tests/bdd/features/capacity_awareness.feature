Feature: Capacity awareness
  As a cluster operator
  I want every admission decision to be observable with capacity figures
  So that I can understand cluster utilisation and debug denials.

  Every decision is recorded as Prometheus metrics (verdict counters, decision
  latency, capacity gauges) and every denial carries the figures a workload owner
  needs to act on without contacting the platform team.

  Background:
    Given the cluster has 100 CPU and 200 GiB allocatable
    And the budget is 80 percent
    And the current allocation is 70 CPU and 110 GiB

  # Spec US2 acceptance scenario 2 — denials are self-explanatory (SC-002)
  Scenario: A denial message names the violated resource and every figure
    When a pod requesting 15 CPU and 10 GiB is submitted
    Then the pod is rejected
    And the rejection message contains "CPU budget exceeded"
    And the rejection message contains "allocated 70000m"
    And the rejection message contains "requested 15000m"
    And the rejection message contains "projected 85000m"
    And the rejection message contains "ceiling 80000m"

  # Spec US2 acceptance scenario 1 — admits are observable too
  Scenario: An admit is recorded as an allow verdict
    When a pod requesting 5 CPU and 40 GiB is submitted
    Then the pod is admitted
    And the metrics contain a CPU allow verdict

  # Spec US2 acceptance scenario 3 — metrics expose verdicts and utilisation
  Scenario: The metrics endpoint records the verdict and capacity gauges
    When a pod requesting 15 CPU and 10 GiB is submitted
    Then the metrics contain a CPU deny verdict
    And the metrics show current CPU allocation of 70000m
    And the metrics show a CPU ceiling of 80000m
    And the metrics show CPU allocation ratio of 0.875

  # Spec US2 acceptance scenario 4 — capacity changes flow into metrics
  Scenario: Capacity changes are reflected in the metrics
    Given the cluster has 200 CPU and 400 GiB allocatable
    And the budget is 80 percent
    And the current allocation is 70 CPU and 110 GiB
    When a pod requesting 5 CPU and 40 GiB is submitted
    Then the metrics show current CPU allocation of 70000m
    And the metrics show a CPU ceiling of 160000m
