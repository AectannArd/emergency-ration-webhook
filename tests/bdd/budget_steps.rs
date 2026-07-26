//! BDD step definitions for budget enforcement (User Story 1, T016).
//!
//! Drives the real `check_budget` decision through Cucumber expressions, so the
//! spec's acceptance scenarios are readable as Gherkin. The `World` holds the
//! mocked allocation figures and the last verdict; a verdict is `Ok(())` for an
//! admission and `Err(AdmissionError)` for a rejection (carrying the contract
//! message we assert on).
//!
//! Run with: `cargo test --test budget_bdd`.

use capacity_admission_webhook::webhook::{AdmissionError, AdmissionVerdict, check_budget};
use cucumber::{World as _, given, then, when};

const GIB: i64 = 1024 * 1024 * 1024;

#[derive(Debug, Default, cucumber::World)]
struct BudgetWorld {
    allocated_cpu: i64,
    allocated_mem: i64,
    ceiling_cpu: i64,
    ceiling_mem: i64,
    existing_cpu: i64,
    response: Option<Result<(), AdmissionError>>,
}

impl BudgetWorld {
    /// Run the budget check and record the verdict (Ok = admit, Err = deny).
    fn record(&mut self, request: (i64, i64)) {
        let verdict = check_budget(
            (self.allocated_cpu, self.allocated_mem),
            request,
            (self.ceiling_cpu, self.ceiling_mem),
        );
        self.response = Some(match verdict {
            AdmissionVerdict::Admit => Ok(()),
            AdmissionVerdict::Deny(violations) => Err(AdmissionError::OverBudget { violations }),
        });
    }

    fn rejection_message(&self) -> String {
        match &self.response {
            Some(Err(err)) => err.message(),
            _ => String::new(),
        }
    }
}

#[given(expr = "the current allocation is {int}m CPU and {int} GiB memory")]
async fn set_allocation(world: &mut BudgetWorld, cpu: i64, mem_gib: i64) {
    world.allocated_cpu = cpu;
    world.allocated_mem = mem_gib * GIB;
}

#[given(expr = "the budget ceiling is {int}m CPU and {int} GiB memory")]
async fn set_ceiling(world: &mut BudgetWorld, cpu: i64, mem_gib: i64) {
    world.ceiling_cpu = cpu;
    world.ceiling_mem = mem_gib * GIB;
}

#[given(expr = "an existing pod consuming {int}m CPU")]
async fn existing_pod(world: &mut BudgetWorld, cpu: i64) {
    world.existing_cpu = cpu;
}

#[when(expr = "a pod requesting {int}m CPU and {int} GiB memory is submitted")]
async fn submit_pod(world: &mut BudgetWorld, cpu: i64, mem_gib: i64) {
    world.record((cpu, mem_gib * GIB));
}

#[when(expr = "the pod is updated to request {int}m CPU")]
async fn update_pod(world: &mut BudgetWorld, cpu: i64) {
    // FR-004: an update is evaluated as the delta (new − existing).
    let delta = cpu - world.existing_cpu;
    world.record((delta, 0));
}

#[then("the pod is admitted")]
async fn admitted(world: &mut BudgetWorld) {
    let Some(result) = &world.response else {
        panic!("no pod has been submitted yet");
    };
    assert!(result.is_ok(), "expected admission, but was rejected");
}

#[then("the pod is rejected")]
async fn rejected(world: &mut BudgetWorld) {
    let Some(result) = &world.response else {
        panic!("no pod has been submitted yet");
    };
    assert!(result.is_err(), "expected rejection, but was admitted");
}

#[then(expr = "the rejection message contains {string}")]
async fn message_contains(world: &mut BudgetWorld, fragment: String) {
    let message = world.rejection_message();
    assert!(
        message.contains(&fragment),
        "rejection message {message:?} does not contain {fragment:?}"
    );
}

#[tokio::main]
async fn main() {
    BudgetWorld::run("tests/bdd/features/budget_enforcement.feature").await;
}
