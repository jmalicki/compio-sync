# Pushing Guide for PRs

## Phase 1 PR (Ready to Push)

**Branch**: `phase1-platform-specific-waiter-queue`
**Status**: Committed locally, needs manual push

### To Push Phase 1:

```bash
# Push with proper GitHub credentials (has workflow scope)
git push -u origin phase1-platform-specific-waiter-queue
```

**Note**: The automated push failed because `.github/workflows/ci.yml` requires `workflow` scope. You'll need to:
1. Use GitHub CLI: `gh auth login` with workflow scope, OR
2. Push from a location with proper OAuth token, OR  
3. Push via SSH if configured

### Create PR:

```bash
gh pr create \
  --base main \
  --head phase1-platform-specific-waiter-queue \
  --title "Phase 1: Platform-Specific Waiter Queue Architecture" \
  --body-file .github/PHASE1_PR.md
```

Or create manually on GitHub using `.github/PHASE1_PR.md` as description.

---

## Phase 2 PR (Stacked - In Progress)

**Branch**: `phase2-linux-io-uring-futex` (based on phase1)
**Status**: Active development

### After Phase 1 is merged:

```bash
# Rebase Phase 2 onto main
git checkout phase2-linux-io-uring-futex
git rebase main

# Then push
git push -u origin phase2-linux-io-uring-futex
```

### Before Phase 1 merges (stacked PR):

```bash
# Push now for review (depends on Phase 1)
git push -u origin phase2-linux-io-uring-futex

# Create PR with Phase 1 as base
gh pr create \
  --base phase1-platform-specific-waiter-queue \
  --head phase2-linux-io-uring-futex \
  --title "Phase 2: Linux io_uring Futex Integration" \
  --body "Depends on #<phase1-pr-number>"
```

---

## Current State

```
main
 └── phase1-platform-specific-waiter-queue (ready to push)
      └── phase2-linux-io-uring-futex (in development)
```

**Next**: Manually push Phase 1, then continue Phase 2 work!

