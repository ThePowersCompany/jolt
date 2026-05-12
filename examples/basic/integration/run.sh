#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../../.." && pwd)"
image="${JOLTR_BASIC_IMAGE:-joltr-basic-example:integration}"
types_dir="$repo_root/target/joltr-basic-integration"
container_id=""

cleanup() {
  if [[ -n "$container_id" ]]; then
    docker rm -f "$container_id" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

if [[ "${JOLTR_BASIC_SKIP_DOCKER_BUILD:-}" != "1" ]]; then
  docker build -f "$repo_root/examples/basic/Dockerfile" -t "$image" "$repo_root"
fi

container_id="$(docker run -d -p 127.0.0.1::3000 "$image")"
mapped_port="$(docker port "$container_id" 3000/tcp)"
host_port="${mapped_port##*:}"
base_url="http://127.0.0.1:$host_port"

ready=0
for _ in {1..60}; do
  if curl -fsS "$base_url/api/test/typed" >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 1
done

if [[ "$ready" != "1" ]]; then
  docker logs "$container_id" || true
  echo "example container did not become ready at $base_url" >&2
  exit 1
fi

mkdir -p "$types_dir"
docker cp "$container_id:/workspace/types.d.ts" "$types_dir/types.d.ts"

cd "$script_dir"
npm run type-check
JOLTR_BASIC_BASE_URL="$base_url" npm test
