import assert from 'node:assert/strict';
import { registerHooks } from 'node:module';
import { test } from 'node:test';

// Workers accept streaming request bodies without Node's required duplex hint.
const NativeRequest = globalThis.Request;
globalThis.Request = class extends NativeRequest {
  constructor(input, init) {
    super(input, init?.body ? { ...init, duplex: 'half' } : init);
  }
};

// Replace only the platform transport; exercise the actual Worker and constructor.
registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier === '@cloudflare/containers') {
      return { shortCircuit: true, url: 'data:text/javascript,' + encodeURIComponent(`
        export class Container { constructor(ctx, env) {} }
        export function getContainer(namespace, name) {
          if (name !== 'main') throw new Error('split room state');
          return namespace;
        }
      `) };
    }
    return nextResolve(specifier, context);
  },
});
const { default: worker, InfiniteChatContainer } = await import('./index.ts');

test('operational secrets reach the container without forwarding other bindings', () => {
  const env = { ADMIN_TOKEN: 'admin-test', METRICS_TOKEN: 'metrics-test',
    NOVA_OPERATOR_TOKEN: 'operator-test', UNRELATED_SECRET: 'never-forward' };
  assert.deepEqual(new InfiniteChatContainer({}, env).envVars, {
    PORT: '3000', RUST_LOG: 'info', ADMIN_TOKEN: 'admin-test',
    METRICS_TOKEN: 'metrics-test', NOVA_OPERATOR_TOKEN: 'operator-test',
  });
  assert.deepEqual(new InfiniteChatContainer({}, { ADMIN_TOKEN: '', METRICS_TOKEN: '' }).envVars,
    { PORT: '3000', RUST_LOG: 'info' });
});

test('HTTP proxy preserves the path, query, body and trusted client address', async () => {
  let received;
  const response = new Response('ok');
  const result = await worker.fetch(new Request('https://example.test/main?x=1', {
    method: 'POST', body: 'payload', headers: { 'CF-Connecting-IP': '192.0.2.1' },
  }), { CHAT_CONTAINER: { fetch: async request => { received = request; return response; } } });
  assert.equal(result, response);
  assert.equal(received.url, 'http://localhost:3000/main?x=1');
  assert.equal(await received.text(), 'payload');
  assert.equal(received.headers.get('CF-Connecting-IP'), '192.0.2.1');
});

test('WebSocket proxy preserves the original handshake', async () => {
  const request = new Request('https://example.test/ws/main', {
    headers: { Upgrade: 'websocket', Origin: 'https://example.test', 'CF-Connecting-IP': '192.0.2.1' },
  });
  await worker.fetch(request, { CHAT_CONTAINER: { fetch: async received => {
    assert.equal(received, request);
    return new Response(null);
  } } });
});

test('unavailable HTTP and WebSocket backends return an uncached retryable response', async () => {
  for (const headers of [{}, { Upgrade: 'websocket' }]) {
    const response = await worker.fetch(new Request('https://example.test/main', { headers }),
      { CHAT_CONTAINER: { fetch: async () => { throw new Error('offline'); } } });
    assert.equal(response.status, 503);
    assert.equal(response.headers.get('Retry-After'), '5');
    assert.equal(response.headers.get('Cache-Control'), 'no-store');
  }
});
