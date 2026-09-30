#!/usr/bin/env bash
set -euo pipefail

package_root="${1:?package root}"
staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT

mkdir -p "$staging/bin" "$staging/libexec/git-core" "$staging/share/git-core/templates"
cp "$(command -v git)" "$staging/bin/git"
cp -a "$(git --exec-path)"/. "$staging/libexec/git-core"/

template_candidates=(
  "$(git --exec-path)/../../share/git-core/templates"
  "/usr/share/git-core/templates"
  "/opt/homebrew/share/git-core/templates"
)
for candidate in "${template_candidates[@]}"; do
  if [[ -d "$candidate" ]]; then
    cp -a "$candidate"/. "$staging/share/git-core/templates"/
    break
  fi
done

version="$(git --version | awk '{print $3}')"
scripts/package-git-runtime.sh "$staging" "$package_root" "$version" "system-package:$RUNNER_OS"
