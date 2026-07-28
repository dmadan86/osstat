<!--
The PR title becomes the commit on main (squash merge), so it must be a valid
Conventional Commit, e.g. "fix(cleaner): resolve symlinks before deleting".
-->

## What

<!-- What does this change do? -->

## Why

<!-- What problem does it solve? Link the issue: "Closes #123". -->

## How it was tested

<!--
Which commands did you actually run, and on which OS? "just ci on Windows" is
useful; "tested locally" is not. Note any platform you could not verify.
-->

## Screenshots

<!-- For UI changes. Delete this section otherwise. -->

## Checklist

- [ ] New code has tests in this PR, not a follow-up
- [ ] `just ci` passes locally
- [ ] No `unwrap()`, `expect()` or `panic!()` outside tests
- [ ] All commits are signed off (`git commit -s`)
- [ ] Documentation updated if behaviour changed

### If this touches deletion, elevation, or cleaning rules

- [ ] The change respects the safety model in ADR-005 (trash by default,
      canonicalize before delete, honour the denylist, write a journal entry)
- [ ] A dry-run branch exists and is covered by fixture tests
- [ ] The elevation helper's command protocol is unchanged, or SECURITY.md and
      ADR-006 are updated in this PR
