# Release Documentation Generation

This document explains how CLI releases automatically include documentation generated from merged pull requests.

## Overview

When a new CLI release is created (by pushing a version tag), the release workflow automatically generates a `RELEASE_NOTES.md` file that contains detailed information about all pull requests merged since the previous release.

## How It Works

### 1. Automatic Trigger
The release documentation generation is triggered when:
- A new tag matching `v*` is pushed (e.g., `v1.0.0`, `v2.1.3`)
- The workflow runs as part of the release process

### 2. PR Detection
The workflow:
1. Identifies the current release tag
2. Finds the previous release tag (or first commit if this is the first release)
3. Scans git history between these two points
4. Extracts all merged pull requests from commit messages

### 3. Information Gathering
For each merged PR, the workflow uses GitHub CLI to fetch:
- **PR Title**: The actual title of the pull request
- **PR Author**: The GitHub username of the PR creator
- **PR Description**: The full description/body of the PR
- **PR Link**: Direct URL to the pull request

If GitHub CLI is unavailable (unlikely in GitHub Actions), it falls back to extracting information from the commit message.

### 4. Document Generation
The workflow creates a `RELEASE_NOTES.md` file with:
- A header explaining the release
- A "What's Changed" section with all PRs
- Each PR entry includes:
  - Title as a markdown header
  - Author attribution
  - Clickable link to the PR
  - Full PR description (if available)
- A footer with a "Full Changelog" link comparing the two tags

### 5. Release Inclusion
The generated `RELEASE_NOTES.md` is:
- Saved in the artifacts directory
- Included with all other release artifacts (CLI binaries, installer scripts)
- Uploaded as part of the GitHub release

## Example Output

```markdown
# Release Notes

This release includes the following changes from merged pull requests.

## What's Changed

### Add new feature for data processing

**Author:** @johndoe  
**PR:** [#42](https://github.com/MattiasHognas/tails/pull/42)

This PR adds a new feature that improves data processing performance by 50%.

Changes include:
- New streaming API
- Optimized memory usage
- Better error handling

---

### Fix bug in authentication

**Author:** @janedoe  
**PR:** [#43](https://github.com/MattiasHognas/tails/pull/43)

Fixes a critical bug in the authentication flow.

---

## Full Changelog

**Full Changelog**: https://github.com/MattiasHognas/tails/compare/v1.0.0...v1.1.0
```

## Benefits

1. **Complete History**: Users can see exactly what changed in each release
2. **Attribution**: Contributors get proper credit for their work
3. **Context**: PR descriptions provide context about why changes were made
4. **Automation**: No manual changelog maintenance required
5. **Consistency**: Same format for every release

## Workflow Configuration

The feature is implemented in `.github/workflows/release-cli.yml` in the `Generate Release Documentation` step.

Key configuration:
- Uses `fetch-depth: 0` to ensure full git history is available
- Sets `GH_TOKEN` environment variable for GitHub CLI authentication
- Runs after artifact downloads but before release creation
- Outputs the generated notes to the workflow log for verification

## Troubleshooting

### No PRs Found
If the release notes show "No merged PRs found in this release":
- Check that PRs were actually merged (not just closed)
- Verify that merge commits use the standard GitHub format: "Merge pull request #XX from branch"
- Ensure git history is available (fetch-depth: 0 is set)

### Missing PR Details
If PR titles show commit messages instead of actual PR titles:
- Verify that the `GH_TOKEN` environment variable is set correctly
- Check that the GitHub CLI has access to the repository
- Review workflow logs for any `gh pr view` errors

### First Release
For the first release (when no previous tag exists):
- The workflow uses the repository's first commit as the baseline
- This will include all PRs merged since the repository was created
- This is expected behavior and ensures nothing is missed

## Maintenance

The release notes generation is fully automated and requires no maintenance. However, if you need to modify the format:

1. Edit `.github/workflows/release-cli.yml`
2. Locate the `Generate Release Documentation` step
3. Modify the markdown template or PR information extraction logic
4. Test changes by pushing a test tag or using workflow_dispatch
5. Review the generated `RELEASE_NOTES.md` in the release artifacts

## Related Files

- `.github/workflows/release-cli.yml` - Main release workflow with documentation generation
- This file is generated and included in every release automatically
