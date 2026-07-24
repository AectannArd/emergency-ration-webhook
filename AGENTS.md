<!-- SPECKIT START -->
For additional context about technologies to be used, project structure,
shell commands, and other important information, read the current plan
<!-- SPECKIT END -->

# Project: emergency-ration-webhook

A Kubernetes admission webhook that tracks cluster capacity (CPU and RAM) and
ensures scheduled workloads do not exceed a configurable capacity percentage.
See `IDEA.md` for the original one-line description; the authoritative spec
lives under `specs/` once `/speckit-specify` has been run.

## Your role: Hermes (specification & planning)

This repository uses **GitHub Spec Kit** for spec-driven development, with a
deliberate split across two machines and two agents:

| Phase | Agent | Machine | How |
|-------|-------|---------|-----|
| Constitution, Clarify, Specify, Plan | **Hermes Agent** | Windows (`D:\development\rust\...`) | speckit-* skills in `~/.hermes/skills/` |
| Tasks, Implement (coding, testing) | **Claude Code** | VM (`~/development/rust/...`) | speckit-* skills in `.claude/skills/` |

You are the **planning agent**. Your job is to drive the upstream half of the
workflow: establish the constitution, clarify ambiguities, write the
specification, and produce the implementation plan. You do **not** write
production code here — implementation is delegated to Claude Code on the VM.

### How to run a spec phase

The speckit skills are plain markdown workflows. Follow them directly against
the `.specify/` project structure:

1. `/speckit-constitution` — fill in `.specify/memory/constitution.md` (currently the unfilled template) with the project's core principles before anything else.
2. `/speckit-clarify` *(optional)* — ask structured questions to de-risk ambiguous areas.
3. `/speckit-specify` — produce `specs/<feature>/spec.md` from the idea.
4. `/speckit-plan` — produce `specs/<feature>/plan.md` from the spec.

### The handoff

Plans written here must reach the VM before Claude Code can implement them.
The repo is the sync mechanism — **commit and push** after each planning
phase, then the VM clone is pulled (or re-cloned) before implementation.

- The `agent-context` extension keeps this `AGENTS.md` and `CLAUDE.md`
  pointing at the most recent `specs/**/plan.md` automatically.
- `.hermes/skills/` is an empty marker — the real speckit skills live in your
  global `~/.hermes/skills/` (installed via `specify integration install
  hermes`). Claude Code's equivalents live in the repo's `.claude/skills/`.

### Script type

Spec Kit scripts in this repo are **bash** (`--script sh`). Planning on Windows
via Hermes does not need to run them directly — Hermes reads the spec templates
and authors the artifacts. The bash scripts are for the VM implementation side.
