<!-- SPECKIT START -->
For additional context about technologies to be used, project structure,
shell commands, and other important information, read the current plan
at specs/009-erw-image-build-automation/plan.md
<!-- SPECKIT END -->

# Project: emergency-ration-webhook

A Kubernetes admission webhook that tracks cluster capacity (CPU and RAM) and
ensures scheduled workloads do not exceed a configurable capacity percentage.
See `IDEA.md` for the original one-line description; the authoritative spec
lives under `specs/` once `/speckit-specify` has been run.

## Your role: Hermes (specification & planning)

This repository uses **GitHub Spec Kit** for spec-driven development, with a
deliberate split across two roles:

| Phase | Agent | Role |
|-------|-------|------|
| Constitution, Clarify, Specify, Plan | **Hermes Agent (you)** | Planning |
| Tasks, Implement (coding, testing) | **Claude Code** | Implementation |

> Machine-specific layout (working directories, OS, skill install paths) is
> intentionally **not** recorded here. Each clone lives wherever the operator
> puts it; the repo is portable and contains no host-specific paths.

You are the **planning agent**. Your job is to drive the upstream half of the
workflow: establish the constitution, clarify ambiguities, write the
specification, and produce the implementation plan. You do **not** write
production code here — implementation is delegated to Claude Code.

### How to run a spec phase

The speckit skills are plain markdown workflows. Follow them directly against
the `.specify/` project structure:

1. `/speckit-constitution` — fill in `.specify/memory/constitution.md` with the project's core principles before anything else.
2. `/speckit-clarify` *(optional)* — ask structured questions to de-risk ambiguous areas.
3. `/speckit-specify` — produce `specs/<feature>/spec.md` from the idea.
4. `/speckit-plan` — produce `specs/<feature>/plan.md` from the spec.

### The handoff

Plans written here must reach the implementation agent before Claude Code can
implement them. The repo is the sync mechanism — **commit and push** after each
planning phase, then the implementation clone is pulled (or re-cloned) before
implementation.

- The `agent-context` extension keeps this `AGENTS.md` and `CLAUDE.md`
  pointing at the most recent `specs/**/plan.md` automatically.
- Skills for each agent are installed by that agent's integration — see the
  relevant `specify integration install` command per machine. The repo carries
  only the empty `.hermes/skills/` marker; Claude Code's equivalents live in
  the repo's `.claude/skills/`.

### Script type

Spec Kit scripts in this repo are **bash** (`--script sh`). Run them from
the project root with `bash .specify/scripts/bash/...` on any POSIX machine.

## Repository Mirroring (GitHub → GitVerse)

**GitHub (`origin`) is the primary upstream.** All development, PRs, CI, and
code review happen on GitHub. **GitVerse (`gitverse`) is a read-only mirror**
that must be kept in sync with every `main` update.

### Rule

After **every** change that lands on `main` — merge of a PR, a direct push, or
a fast-forward — the `main` branch must also be pushed to GitVerse.

```bash
git push gitverse main
```

### Automation (dual push URLs)

Clones may configure `origin` to push to both upstreams automatically:

```bash
git remote set-url --add --push origin git@github.com:AectannArd/emergency-ration-webhook.git
git remote set-url --add --push origin ssh://git@gitverse.ru:2222/AectannArd/emergency-ration-webhook.git
```

After this, a single `git push origin main` pushes to both. Feature branches
and spec branches are pushed to GitHub only (GitVerse mirrors `main`).

### GitVerse remote

```
gitverse  ssh://git@gitverse.ru:2222/AectannArd/emergency-ration-webhook.git
```
