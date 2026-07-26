Feature: Fail-safe operation
  As a cluster operator
  I want the webhook to reject any admission it cannot authoritatively verify
  So that the cluster is never overcommitted under degraded knowledge.

  When capacity data is stale or missing, or the admission request is malformed,
  or a resource quantity cannot be parsed, the admission is rejected
  (allowed: false). There is no path that admits under degraded knowledge.

  Background:
    Given the cluster capacity is 100 CPU and 200 GiB at 80 percent budget

  # Spec US3 acceptance scenario 1
  Scenario: Stale capacity data is rejected
    Given the allocation was last refreshed 60 seconds ago
    And the current allocation is 70 CPU and 110 GiB
    When a pod requesting 5 CPU and 40 GiB is submitted
    Then the pod is rejected
    And the rejection message contains "capacity data unavailable"
    And the rejection message contains "exceeds 30s threshold"

  # Spec US3 acceptance scenario 2
  Scenario: Missing allocation state is rejected
    Given the allocation state is not populated
    When a pod requesting 5 CPU and 40 GiB is submitted
    Then the pod is rejected
    And the rejection message contains "allocation state not initialised"

  Scenario: Missing cluster capacity is rejected
    Given the cluster capacity is not populated
    And the allocation was last refreshed 5 seconds ago
    And the current allocation is 70 CPU and 110 GiB
    When a pod requesting 5 CPU and 40 GiB is submitted
    Then the pod is rejected
    And the rejection message contains "cluster capacity state not initialised"

  # Spec US3 acceptance scenario 3
  Scenario: A malformed admission request is rejected
    Given the admission request is malformed
    When it is submitted
    Then the pod is rejected
    And the rejection message contains "admission request malformed"

  Scenario: An unparseable resource quantity is rejected
    Given a pod requests an unparseable CPU quantity
    When it is submitted
    Then the pod is rejected
    And the rejection message contains "cannot parse resource quantity"
