# Publish a GitHub Release

Publish a draft release for the Narrator app. The argument should be a tag name (e.g. `v0.3.0`). If no argument is provided, use the latest tag.

## Steps

### 1. Resolve the tag
- If `$ARGUMENTS` is provided, use it as the tag name
- Otherwise, run `git describe --tags --abbrev=0` to get the latest tag
- Confirm the tag exists: `git tag -l <tag>`

### 2. Verify the build is green
- Run `gh run list --branch <tag> --limit 1 --json status,conclusion,name` to find the release workflow run
- If the run is still in progress, report status and ask the user to wait
- If any job failed, fetch the failure logs with `gh run view <run_id> --log-failed` and report the issue — do NOT proceed
- Only continue if all jobs succeeded

### 3. Verify release assets
- Run `gh release view <tag> --json assets,isDraft --jq '{draft: .isDraft, assets: [.assets[].name]}'`
- Confirm the release exists and is a draft
- Check that expected assets are present:
  - macOS: `*_aarch64.dmg`, `*_x64.dmg`, `*.app.tar.gz` (both arches), `.sig` files
  - Windows: `*_x64-setup.exe`, `*_x64_en-US.msi`, `.nsis.zip`, `.sig` files
  - `latest.json` (required for auto-update)
- Report any missing assets as warnings

### 4. Rename installer artifacts
Rename only the user-facing installers to friendly names. Do NOT rename `.tar.gz`, `.sig`, `.nsis.zip`, `.msi.zip`, or `latest.json` files — those are used by the auto-updater.

Use the GitHub API to rename:
```
Narrator_<version>_aarch64.dmg  →  Narrator-<version>-macOS-Apple-Silicon.dmg
Narrator_<version>_x64.dmg     →  Narrator-<version>-macOS-Intel.dmg
Narrator_<version>_x64-setup.exe  →  Narrator-<version>-Windows-x64-setup.exe
Narrator_<version>_x64_en-US.msi  →  Narrator-<version>-Windows-x64.msi
```

Get the release ID and assets:
```bash
RELEASE_ID=$(gh api repos/roalexandru/VideoNarrator/releases --jq '.[] | select(.tag_name=="<tag>") | .id' | head -1)
```

For each rename, find the asset by original name suffix, then:
```bash
gh api --method PATCH repos/roalexandru/VideoNarrator/releases/assets/<asset_id> -f name="<new_name>" --silent
```

### 5. Generate release notes
- Run `git log <previous_tag>...<tag> --oneline` to get all commits since the last release
- Read the key changed files to understand what's new
- Write release notes in this format:

```markdown
## Download

| Platform | Installer |
|----------|-----------|
| macOS (Apple Silicon) | `Narrator-<version>-macOS-Apple-Silicon.dmg` |
| macOS (Intel) | `Narrator-<version>-macOS-Intel.dmg` |
| Windows | `Narrator-<version>-Windows-x64-setup.exe` |

> **macOS note:** The app is not code-signed. After installing, run `sudo xattr -rd com.apple.quarantine /Applications/Narrator.app` in Terminal.

## What's New in <tag>

<summarize the key changes from the commits — group by feature, use bold headers for major items>
```

### 6. Update release and publish
- Show the user the generated release notes and ask for approval before publishing
- Apply with: `gh release edit <tag> --title "Narrator <tag>" --notes "<notes>" --draft=false`
- Confirm the release is published by checking `gh release view <tag> --json isDraft`
- Print the release URL
