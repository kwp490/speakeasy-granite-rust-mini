# AI working agreement

This is the shared operating agreement for coding agents. `AGENTS.md` and
`CLAUDE.md` point here so tool-specific instructions do not drift.

Platform instructions take precedence. The user's current request defines the
authorized task; repository documentation never expands that authority.

## Scope and evidence

Before editing:

1. Run `git status -sb` and preserve existing user changes.
2. Trace the production path, its callers, and its consumers. Names, comments,
   handoff notes, and tests are leads, not proof that a path is live.
3. Establish the relevant baseline: behavior, payload, test selection,
   documentation claim, or measurement.
4. Stop for a user decision if an assumption would materially change behavior,
   privacy, security, compatibility, installation, or release scope.

Distinguish evidence when it matters:

- **Observed:** directly verified from code, output, a manifest, or an executed
  test.
- **Inferred:** supported by observed facts but not directly exercised.
- **Unverified:** blocked on hardware, artifacts, permissions, credentials, a
  clean environment, or another external condition.

Do not call work confirmed, fixed, safe, dead, covered, or release-ready when
the necessary evidence remains inferred or unverified.

Keep the requested batch narrow. Report adjacent findings instead of silently
implementing them. Review and diagnosis do not authorize edits; code changes do
not authorize versioning, installer work, publication, or release activity.

## Changes, Git, and external state

- Prefer the smallest change that establishes the requested invariant. Separate
  behavior, broad refactoring, documentation cleanup, infrastructure, and
  release preparation unless the task explicitly combines them.
- Do not create or switch branches, commit, amend, merge, rebase, tag, push,
  publish, release, or delete refs without current user authorization.
- Never discard a working file with `git checkout --`, `git restore`,
  `git reset --hard`, or an equivalent command. Copy it aside for test controls
  or use a narrowly scoped patch.
- Do not stop processes, write to the real registry, install tools, fetch large
  artifacts, or run hardware proofs unless required by the authorized task.
- Resolve and verify exact targets before deletion or replacement.
- Determine synchronization from Git, not prose: `git status -sb` and
  `git log --oneline origin/main..HEAD`.

When a commit is authorized, keep one coherent commit per requested batch unless
the user asks otherwise. Report its SHA and whether it was pushed.

## Tests

- A regression test must reach the production decision or boundary that caused
  the defect.
- Prove each new regression test with a faithful red control that restores the
  real defect. A control that stays green is defective until explained. Restore
  files from safe copies, then rerun green — and check that the restored file
  was actually rebuilt, because a copy carries its original timestamp and a
  build that fingerprints on timestamps will skip it.
- Prefer behavioral tests. Source assertions are appropriate for structural
  policy—configuration, IPC allowlists, forbidden symbols, and public
  disclosures—but do not replace runtime behavior.
- Mocks and injected dependencies must observe the production boundary rather
  than reproduce the implementation's answer.
- Verify ambiguous test filters with `--list` and report the tests selected.
- Run verification in proportion to risk. Cross-cutting, concurrency,
  persistence, privacy, security, installer, and release-sensitive changes need
  the full gate unless a concrete limitation is reported.

Report passed tests separately from ignored or unselected tests. Name missing
prerequisites and the actual worker, model, runtime, device, profile, and
artifact used for hardware or performance claims. Incremental builds do not
prove a clean clone.

## Documentation and high-risk claims

- Document current behavior and durable constraints, not the debugging session.
  Open state belongs in `docs/handoff/CURRENT.md`; resolved history belongs in
  Git.
- Do not record volatile push state, commit counts, test totals, file dates, or
  local machine state. Give the command that obtains the current answer.
- Verify public claims against production code, manifests, build output, or an
  executed proof. Search for equivalent wording when correcting a claim.
- Avoid absolutes such as “always,” “never,” “every,” and “safe” unless all
  relevant paths support them. Put limitations beside the protection.
- Keep production comments concise and current. Do not cite numbered handoff
  items or add incident chronology.
- Update `docs/UI-GUIDE.md` with visible behavior.

Treat privacy, security, installer, distribution, licensing, and release claims
as high risk. Do not turn one machine, region, provider, or build into a general
guarantee; do not present one configuration as safer while ignoring its other
data paths; and record third-party terms without offering a legal conclusion.

Version bumps, changelogs, installer proofs, uploads, tags, and releases require
separate authorization. A green default gate does not replace ignored hardware
proofs or installer lifecycle proofs.

## Report

Report:

- the outcome and files or behavior changed;
- behavior deliberately not changed;
- commands and exact results;
- red controls for new regression tests;
- ignored, unselected, or unavailable proofs;
- remaining risks, assumptions, and owner decisions;
- branch, working-tree, commit, and push state obtained from Git.

Lead with failures and limitations before the green gate. Keep the report short
enough that its caveats remain visible.
