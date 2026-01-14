import { Container } from '@cloudflare/containers';
import { getContainer } from '@cloudflare/containers';

/**
 * Infinite Chat Container - Rust WebSocket chat server
 * Proxies HTTP and WebSocket connections to the Rust container
 */
export class InfiniteChatContainer extends Container {
  // Port the Rust server listens on
  defaultPort = 3000;
  
  // Keep container active for 30 minutes without requests
  sleepAfter = '30m';
  
  // Environment variables for the container
  envVars = {
    PORT: '3000',
    RUST_LOG: 'info',
  };
  
  // Health check endpoint
  pingEndpoint = '/health';
}

interface Env {
  CHAT_CONTAINER: DurableObjectNamespace<InfiniteChatContainer>;
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const pathname = url.pathname;
    
    // Get container instance - use 'main' as default singleton for all traffic
    // Each room is handled internally by the Rust server
    const container = getContainer(env.CHAT_CONTAINER, 'main');
    
    // Check if this is a WebSocket upgrade request
    const upgradeHeader = request.headers.get('Upgrade');
    const isWebSocket = upgradeHeader?.toLowerCase() === 'websocket';
    
    // WebSocket connections to /ws/:room
    if (isWebSocket && pathname.startsWith('/ws/')) {
      console.log(`Proxying WebSocket connection to ${pathname}`);
      
      // Create a new request with the same headers for the container
      const wsRequest = new Request(`http://localhost:3000${pathname}`, {
        method: request.method,
        headers: request.headers,
      });
      
      return container.fetch(wsRequest);
    }
    
    // All other HTTP requests (static pages, health, metrics, etc.)
    // Forward to the Rust server
    const httpRequest = new Request(`http://localhost:3000${pathname}${url.search}`, {
      method: request.method,
      headers: request.headers,
      body: request.body,
    });
    
    return container.fetch(httpRequest);
  },
};
