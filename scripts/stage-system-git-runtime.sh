#!/usr/bin/env bash
set -euo pipefail

package_root="${1:?package root}"
staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT

mkdir -p "$staging/bin" "$staging/libexec/git-core" "$staging/share/git-core/templates"
cp "$(command -v git)" "$staging/bin/git"
exec_path="$(git --exec-path)"
cp -a "$exec_path"/. "$staging/libexec/git-core"/
# Homebrew links some helpers outside git-core (git-cvsserver -> ../../bin/...).
# Replace absolute or dangling links with their targets; drop links already broken.
while IFS= read -r -d '' link; do
  if [[ "$(readlink "$link")" != /* && -e "$link" ]]; then
    continue
  fi
  source="$exec_path/${link#"$staging/libexec/git-core/"}"
  rm "$link"
  if [[ -e "$source" ]]; then
    cp -RL "$source" "$link"
  fi
done < <(find "$staging/libexec/git-core" -type l -print0)

template_candidates=(
  "$exec_path/../../share/git-core/templates"
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
bash scripts/package-git-runtime.sh "$staging" "$package_root" "$version" "system-package:$RUNNER_OS"
