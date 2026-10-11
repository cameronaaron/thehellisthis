import { Container, getContainer } from '@cloudflare/containers';

/**
 * The Rust WebSocket chat server, in a Cloudflare Container.
 *
 * The Worker is deliberately thin: it routes and tells the container who the
 * client is. Rooms, identity and rate limiting are the container's, because
 * that state lives in its memory.
 */
export class InfiniteChatContainer extends Container<Env> {
  defaultPort = 3000;

  envVars: Record<string, string>;

  // Polled for health; the handler never takes the room write lock, so a busy
  // server can't look dead.
  pingEndpoint = '/health';

  // Stop after this long with no in-flight request. An open WebSocket keeps its
  // proxying `fetch()` in flight, so a live conversation never trips it. The
  // library default, stated so it reads as a decision.
  sleepAfter = '10m';

  // Assigned here because Worker secrets are only reachable through `env`.
  // Bindings don't become container environment variables on their own, so
  // each is forwarded explicitly, and omitted (not sent empty) when unset, so
  // the server's checks fail closed and refuse every request to that endpoint.
  constructor(ctx: Container<Env>['ctx'], env: Env) {
    super(ctx, env);
    this.envVars = {
      PORT: '3000',
      RUST_LOG: 'info',
      ...(env.ADMIN_TOKEN ? { ADMIN_TOKEN: env.ADMIN_TOKEN } : {}),
      ...(env.METRICS_TOKEN ? { METRICS_TOKEN: env.METRICS_TOKEN } : {}),
      ...(env.NOVA_OPERATOR_TOKEN ? { NOVA_OPERATOR_TOKEN: env.NOVA_OPERATOR_TOKEN } : {}),
    };
  }
}

interface Env {
  CHAT_CONTAINER: DurableObjectNamespace<InfiniteChatContainer>;
  // Secrets from `wrangler secret put`, never plaintext `vars`. Optional
  // because local `wrangler dev` usually runs without them.
  NOVA_OPERATOR_TOKEN?: string;
  ADMIN_TOKEN?: string;
  METRICS_TOKEN?: string;
}

/**
 * One instance holds every room in memory, so all traffic must reach it; a
 * second would be a separate set of rooms behind the same URLs.
 */
const CONTAINER_INSTANCE = 'main';

/**
 * Tells the container who the client is. It reads `CF-Connecting-IP`, then
 * `X-Forwarded-For`; without them every visitor is one address and the
 * per-address limits become global.
 */
function withClientAddress(request: Request): Headers {
  const headers = new Headers(request.headers);
  const clientIp = request.headers.get('CF-Connecting-IP');

  if (clientIp) {
    headers.set('CF-Connecting-IP', clientIp);
    // Prepend, keeping any upstream chain.
    const forwarded = headers.get('X-Forwarded-For');
    headers.set('X-Forwarded-For', forwarded ? `${clientIp}, ${forwarded}` : clientIp);
  }

  return headers;
}

function unavailable(): Response {
  return new Response('Chat server unavailable', {
    status: 503,
    headers: {
      'Retry-After': '5',
      'Cache-Control': 'no-store',
      'Content-Type': 'text/plain; charset=utf-8',
    },
  });
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const container = getContainer(env.CHAT_CONTAINER, CONTAINER_INSTANCE);

    // WebSocket upgrades pass through untouched: rebuilding the request would
    // drop the handshake headers, and the container checks the Origin itself.
    if (request.headers.get('Upgrade')?.toLowerCase() === 'websocket') {
      try {
        return await container.fetch(request);
      } catch {
        return unavailable();
      }
    }

    // Caching happens at the edge (`cache.enabled` in wrangler.jsonc) before
    // this runs, honouring the container's Cache-Control: `immutable` for
    // `/app.js`, `no-store` elsewhere (`src/startup.rs::build_router`). This
    // replaced a path-keyed `caches.default` that ignored that header.
    const proxied = new Request(`http://localhost:3000${url.pathname}${url.search}`, {
      method: request.method,
      headers: withClientAddress(request),
      body: request.body,
    });

    try {
      return await container.fetch(proxied);
    } catch {
      return unavailable();
    }
  },
};
