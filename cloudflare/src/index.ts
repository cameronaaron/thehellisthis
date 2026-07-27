import { Container, getContainer } from '@cloudflare/containers';

/**
 * The Rust WebSocket chat server, running in a Cloudflare Container.
 *
 * The Worker in front of it is deliberately thin: it routes, it tells the
 * container who the client is, and it caches the one static asset. Everything
 * else — rooms, identity, rate limiting — is the container's job, because all
 * of that state lives in the container's memory.
 */
export class InfiniteChatContainer extends Container {
  defaultPort = 3000;

  envVars = {
    PORT: '3000',
    RUST_LOG: 'info',
  };

  // Polled by the runtime to decide whether this instance is healthy. The
  // handler never takes the room write lock, so a busy server cannot look dead.
  pingEndpoint = '/health';
}

interface Env {
  CHAT_CONTAINER: DurableObjectNamespace<InfiniteChatContainer>;
}

/**
 * A single container instance holds every room in memory, so all traffic must
 * reach the same one. A second instance would be a second, separate set of
 * rooms answering the same URLs.
 */
const CONTAINER_INSTANCE = 'main';

/** Immutable, content-addressed, and identical for every visitor. */
const CACHEABLE_PATHS = new Set(['/app.js']);

/**
 * Tells the container who the client is.
 *
 * The container reads `CF-Connecting-IP` first and falls back to
 * `X-Forwarded-For`; both are set here so its per-IP connection limits see real
 * clients. Without this every visitor arrives as the same address and those
 * limits are global rather than per-IP.
 */
function withClientAddress(request: Request): Headers {
  const headers = new Headers(request.headers);
  const clientIp = request.headers.get('CF-Connecting-IP');

  if (clientIp) {
    headers.set('CF-Connecting-IP', clientIp);
    // Append rather than overwrite, preserving any upstream chain.
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
      'Content-Type': 'text/plain; charset=utf-8',
    },
  });
}

export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    const url = new URL(request.url);
    const container = getContainer(env.CHAT_CONTAINER, CONTAINER_INSTANCE);

    // WebSocket upgrades are passed through untouched. Rebuilding the request
    // would drop the handshake headers the upgrade depends on, and the
    // container validates the Origin itself.
    if (request.headers.get('Upgrade')?.toLowerCase() === 'websocket') {
      try {
        return await container.fetch(request);
      } catch {
        return unavailable();
      }
    }

    // The script is immutable and content-addressed, so it can be served from
    // the edge cache and never reach the container twice.
    const isCacheable = CACHEABLE_PATHS.has(url.pathname);
    if (isCacheable) {
      const cached = await caches.default.match(request);
      if (cached) return cached;
    }

    const proxied = new Request(`http://localhost:3000${url.pathname}${url.search}`, {
      method: request.method,
      headers: withClientAddress(request),
      body: request.body,
    });

    let response: Response;
    try {
      response = await container.fetch(proxied);
    } catch {
      return unavailable();
    }

    if (isCacheable && response.ok) {
      // Cache without blocking the response the visitor is waiting for.
      ctx.waitUntil(caches.default.put(request, response.clone()));
    }

    return response;
  },
};
