<!-- SPECKIT START -->
For additional context about technologies to be used, project structure,
shell commands, and other important information, read the current plan
at specs/001-capacity-admission-webhook/plan.md
<!-- SPECKIT END -->

# Project: emergency-ration-webhook

A Kubernetes admission webhook that tracks cluster capacity (CPU and RAM) and
ensures scheduled workloads do not exceed a configurable capacity percentage.

## Your role: Claude Code (implementation)

This repository uses **GitHub Spec Kit** with a deliberate two-agent split:

| Phase | Agent | Role |
|-------|-------|------|
| Constitution, Clarify, Specify, Plan | Hermes Agent | Planning |
| Tasks, Implement (coding, testing) | **Claude Code (you)** | Implementation |

> Machine-specific layout is intentionally **not** recorded in this repo. Work
> from the clone wherever it lives on this host; the repo contains no
> host-specific paths.

You are the **implementation agent**. The specification, plan, and constitution
are produced upstream by Hermes and arrive here via `git`. Your job is to turn
the plan into code: `/speckit-tasks` then `/speckit-implement`.

### How to run an implementation phase

1. Ensure you are on the latest `main`: `git pull`.
2. Read the current plan referenced in the SPECKIT block above, and
   `.specify/memory/constitution.md` for the non-negotiable principles.
3. `/speckit-tasks` — generate actionable tasks from the plan.
4. `/speckit-implement` — execute the tasks, write the code, run tests.

Do not re-specify or re-plan from scratch unless the plan is genuinely missing
or inconsistent — that is Hermes's responsibility. If the plan is ambiguous,
surface it as a question rather than inventing scope.

### Script type

Spec Kit scripts in this repo are **bash** (`--script sh`). Run them from the
project root with `bash .specify/scripts/bash/...`.
