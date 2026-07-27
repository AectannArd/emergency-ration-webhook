Feature: Schedulable node filter
  As a cluster operator
  I want the capacity aggregate to exclude cordoned and control-plane nodes
  So that the budget reflects capacity kube-scheduler can actually place on.

  Cordoned nodes (spec.unschedulable = true) are excluded by default; an optional
  ClusterCapacity spec.nodeSelector excludes arbitrary node subsets by label. The
  status reports how many nodes were excluded and why (spec-006).

  # US1 — cordoned nodes excluded by default (P1, the phantom-capacity fix)
  @cordon
  Scenario: Cordoned node is excluded from capacity
    Given a cluster with 3 schedulable nodes each with 16 CPU and 32Gi memory
    When one node is cordoned
    Then the status reports nodeCount 2
    And the excludedByUnschedulable count is 1
    And the excludedBySelector count is 0

  # US2 — label-selector exclusion (P2)
  @selector
  Scenario: Control-plane nodes excluded by label selector
    Given a cluster with 2 worker nodes and 1 control-plane node
    And the nodeSelector excludes nodes labeled "node-role.kubernetes.io/control-plane"
    Then the status reports nodeCount 2
    And the excludedBySelector count is 1
    And the excludedByUnschedulable count is 0

  # US3 — observability of excluded nodes (P3)
  @observability
  Scenario: Status shows the excluded-node breakdown
    Given a cluster with 5 nodes where 1 is cordoned and 1 matches the nodeSelector
    And the nodeSelector excludes nodes labeled "node-role.kubernetes.io/control-plane"
    When the controller reconciles
    Then the status reports nodeCount 3
    And the excludedNodeCount is 2
    And the excludedByUnschedulable count is 1
    And the excludedBySelector count is 1
