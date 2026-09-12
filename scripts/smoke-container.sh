#!/bin/bash
# Verify the shipped image, including its runtime libraries and embedded assets.
set -euo pipefail
cd "$(dirname "$0")/.."

# Wrangler deploys linux/amd64 even when the developer uses an ARM Mac.
docker build --platform linux/amd64 --tag infinite-chat:smoke .
container=$(docker run --platform linux/amd64 --detach --publish 127.0.0.1::3000 --memory 256m \
    --env ADMIN_TOKEN=container-smoke-admin --env METRICS_TOKEN=container-smoke-metrics \
    infinite-chat:smoke)
trap 'docker rm -f "$container" >/dev/null' EXIT
port=$(docker port "$container" 3000/tcp)
port=${port##*:}

if ! PORT="$port" python3 - <<'PY'
import base64
import os
import time
import urllib.error
import urllib.request

base = 'http://127.0.0.1:' + os.environ['PORT']
for attempt in range(100):
    try:
        with urllib.request.urlopen(base + '/health', timeout=1) as response:
            assert response.status == 200
        break
    except (OSError, urllib.error.URLError):
        time.sleep(0.1)
else:
    raise AssertionError('container did not become healthy')

for path, content_type in [('/health', 'application/json'), ('/main', 'text/html'),
                           ('/app.js', 'javascript'), ('/nova.js', 'javascript'),
                           ('/nova_wasm_bg.wasm', 'application/wasm')]:
    with urllib.request.urlopen(base + path, timeout=5) as response:
        body = response.read()
        assert response.status == 200 and body, path
        assert content_type in response.headers['Content-Type'], path
        assert response.headers['X-Content-Type-Options'] == 'nosniff', path
        if path.endswith('.wasm'):
            assert body[:4] == b'\0asm', 'invalid WASM artifact'

for path, status, credential in [
    ('/admin', 401, 'Basic ' + base64.b64encode(b'admin:container-smoke-admin').decode()),
    ('/metrics', 404, 'Bearer container-smoke-metrics'),
]:
    try:
        urllib.request.urlopen(base + path, timeout=5)
        raise AssertionError(path + ' accepted unauthenticated request')
    except urllib.error.HTTPError as error:
        assert error.code == status, (path, error.code)
    request = urllib.request.Request(base + path, headers={'Authorization': credential})
    with urllib.request.urlopen(request, timeout=5) as response:
        assert response.status == 200, path
        assert response.headers['Cache-Control'] == 'no-store', path
print('Container HTTP, assets and operational authentication passed.')
PY
then
    docker logs "$container"
    exit 1
fi

test "$(docker inspect --format '{{.Config.User}}' "$container")" = nonroot
docker stop --time 10 "$container" >/dev/null
test "$(docker inspect --format '{{.State.ExitCode}}' "$container")" = 0
echo 'Container non-root runtime and graceful shutdown passed.'
