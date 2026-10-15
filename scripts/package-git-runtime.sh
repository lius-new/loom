#!/usr/bin/env bash
set -euo pipefail

source_root="${1:?source Git runtime root}"
package_root="${2:?package root}"
version="${3:?Git version}"
source_url="${4:?upstream source URL}"
runtime="$package_root/runtime/git"

mkdir -p "$runtime"
cp -a "$source_root"/. "$runtime"/
printf '%s\n' "$version" > "$runtime/VERSION"

python3 - "$runtime" "$version" "$source_url" <<'PY'
import hashlib, json, os, platform, sys
root, version, source = sys.argv[1:]
files = []
for base, _, names in os.walk(root):
    for name in sorted(names):
        path = os.path.join(base, name)
        relative = os.path.relpath(path, root).replace(os.sep, '/')
        if relative == 'MANIFEST.json':
            continue
        with open(path, 'rb') as handle:
            digest = hashlib.sha256(handle.read()).hexdigest()
        files.append({'path': relative, 'sha256': digest})
manifest = {
    'version': version,
    'platform': platform.system(),
    'architecture': platform.machine(),
    'source': source,
    'files': sorted(files, key=lambda item: item['path']),
}
with open(os.path.join(root, 'MANIFEST.json'), 'w', encoding='utf-8') as handle:
    json.dump(manifest, handle, ensure_ascii=False, indent=2)
PY
