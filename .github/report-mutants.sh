#!/usr/bin/env bash
# Report cargo-mutants results as a sticky PR comment (advisory). Reads
# app/mutants.out; never fails the build. Expects the GitHub Actions environment
# (GH_TOKEN, GITHUB_REPOSITORY, GITHUB_REF).
#
# Sticky behavior, chosen to stay calm under the non-determinism of mutation runs:
#   - surviving mutants -> create-or-update the comment with the list.
#   - clean run         -> UPDATE an existing comment to a clean status (so a stale
#                          survivor list never lingers) but never CREATE one (a PR
#                          that's always clean stays unmarked) and never DELETE
#                          (delete+recreate would flicker and re-notify). The
#                          footer makes clear a clean result is per-run, not proof
#                          a previously-flagged mutant was fixed.
set -euo pipefail

OUT=app/mutants.out
missed=0
caught=0
[ -f "$OUT/missed.txt" ] && missed=$(grep -c . "$OUT/missed.txt" || true)
[ -f "$OUT/caught.txt" ] && caught=$(grep -c . "$OUT/caught.txt" || true)
total=$((missed + caught))
marker='<!-- ironstate-mutants -->'
title='### 🧬 Mutation testing (advisory, `--in-diff`)'
footer='_Advisory and per-run: results can vary with build timing, so a clean result reflects this run — not proof a previously-flagged mutant was fixed._'

if [ "$missed" -gt 0 ]; then
  middle=$(cat <<EOF
**$missed surviving** of $total tested mutant(s) in the changed Rust.

<details><summary>Surviving mutants — a change no test caught (tighten a test, or annotate \`#[mutants::skip]\` if equivalent)</summary>

\`\`\`
$(head -40 "$OUT/missed.txt")
\`\`\`
</details>
EOF
)
elif [ "$total" -gt 0 ]; then
  middle="✅ All $total mutant(s) in the changed Rust were caught."
else
  middle="✅ No mutable changes in the current diff."
fi

body="$marker
$title

$middle

$footer"

printf '%s\n' "$body" >> "${GITHUB_STEP_SUMMARY:-/dev/stdout}"

repo="$GITHUB_REPOSITORY"
pr=$(printf '%s' "$GITHUB_REF" | cut -d/ -f3)
cid=$(gh api "repos/$repo/issues/$pr/comments" \
  --jq "map(select(.body|startswith(\"$marker\")))|.[0].id // empty")
if [ -n "$cid" ]; then
  gh api -X PATCH "repos/$repo/issues/comments/$cid" -f body="$body" >/dev/null
  echo "updated PR comment ($missed surviving)"
elif [ "$missed" -gt 0 ]; then
  gh api -X POST "repos/$repo/issues/$pr/comments" -f body="$body" >/dev/null
  echo "posted PR comment ($missed surviving)"
else
  echo "clean, no existing comment — leaving the PR unmarked"
fi
