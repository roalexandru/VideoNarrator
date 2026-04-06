# Create a Production-Ready Pull Request

Review all changes on the current branch, fix issues, and create a PR to main. This is a thorough review — treat it as a senior architect + security reviewer.

## Phase 1: Understand the Changes

1. Run `git log main..HEAD --oneline` to see all commits on this branch
2. Run `git diff main...HEAD --stat` to see all changed files
3. Read every changed file to understand the full scope

## Phase 2: Architecture & Code Review

Review as a **Tauri v2 + TypeScript architect**:

### Rust Backend
- Are Tauri commands properly structured (error handling, state access, async)?
- Are new dependencies justified? Check `Cargo.toml` for unnecessary additions
- Is file I/O safe (proper error handling, no panics in production paths)?
- Are new modules registered in `lib.rs`?
- Does `cargo clippy -- -D warnings` pass?
- Does `cargo fmt -- --check` pass?

### React Frontend
- Are components following existing patterns (inline styles, design tokens from `src/lib/theme.ts`)?
- Are Tauri commands wrapped in `src/lib/tauri/commands.ts` (never raw `invoke()` in components)?
- Is state management correct (Zustand stores, no prop drilling for global state)?
- Are there memory leaks (missing cleanup in useEffect, dangling listeners)?
- Does `pnpm typecheck` pass?

### General
- Are there any TODO/FIXME/HACK comments that should be resolved?
- Is the code DRY without being over-abstracted?
- Are error messages user-friendly?

**Fix any issues found before proceeding.**

## Phase 3: Security Review

Review for security vulnerabilities:

- **Command injection:** Are user inputs sanitized before passing to shell commands or ffmpeg?
- **Path traversal:** Are file paths validated? Can users escape `~/.narrator/`?
- **XSS:** Is user content rendered safely in React (no dangerouslySetInnerHTML)?
- **API key exposure:** Are keys only stored in `~/.narrator/config.json` with restricted permissions? Never logged or sent to telemetry?
- **CSP compliance:** Does `tauri.conf.json` CSP allow only necessary origins?
- **Dependency audit:** Run `pnpm audit` and `cargo audit` (if available). Flag high/critical issues.
- **Tauri permissions:** Does `capabilities/default.json` follow least-privilege?

**Fix any issues found before proceeding.**

## Phase 4: Open Source Review

- Is the MIT license respected? No proprietary dependencies?
- Are there hardcoded secrets, API keys, or internal URLs in the code? (Aptabase key is fine — it's a public app key)
- Is `SECURITY.md` still accurate?
- Would a new contributor understand this code?

**Fix any issues found before proceeding.**

## Phase 5: Test Coverage

### Unit Tests
- Run `pnpm test` — all tests must pass
- Check if new/changed components have corresponding tests in `src/__tests__/`
- Check if new/changed Zustand stores have tests in `src/stores/*.test.ts`
- Check if new Tauri commands are mocked in `src/__tests__/setup.ts`
- **Write missing tests** following existing patterns (use `setupDefaultMocks`, `resetAllStores`, `mockIPC`)

### Rust Tests
- Run `cargo test --manifest-path src-tauri/Cargo.toml`
- Check if new Rust modules have tests

### Test Quality
- Do tests verify behavior, not implementation details?
- Are edge cases covered (empty inputs, errors, boundary conditions)?
- Are async operations properly awaited in tests?

**Write any missing tests before proceeding.**

## Phase 6: In-Product Help & Documentation

- If new features were added, check if `src/features/help/HelpPanel.tsx` needs updating
- If new settings were added, check if descriptions are clear in the Settings panel
- If new export formats or capabilities were added, check if the help sections cover them
- If the Privacy Policy or Terms of Service are affected, update `src/features/legal/`

**Update any stale documentation before proceeding.**

## Phase 7: Final Verification

Run the full quality gate locally:
```bash
pnpm typecheck
pnpm test
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

All must pass. If any fail, fix and re-run.

## Phase 8: Report & Wait for GO

Present a summary to the user:

```
## PR Review Summary

### Changes
- <bullet list of what changed>

### Review Findings
- **Architecture:** <pass/issues found and fixed>
- **Security:** <pass/issues found and fixed>  
- **Open Source:** <pass/issues found and fixed>
- **Tests:** <X tests total, Y new — all passing>
- **Help/Docs:** <up to date / updated>

### Quality Gate
- TypeScript: ✓
- Unit tests: ✓ (X/X passed)
- Rust fmt: ✓
- Rust clippy: ✓
- Rust tests: ✓

Ready to create PR. Proceed?
```

**STOP and wait for the user to say GO.** Do not create the PR until explicitly told to proceed.

## Phase 9: Create the PR (only after user GO)

1. Push the branch: `git push -u origin HEAD`
2. Create the PR with `gh pr create`:
   - Title: concise, under 70 characters
   - Body: summary of changes, test plan, review checklist
3. Return the PR URL
