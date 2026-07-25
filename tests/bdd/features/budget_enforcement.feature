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
