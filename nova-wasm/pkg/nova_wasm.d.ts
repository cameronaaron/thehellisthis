/* tslint:disable */
/* eslint-disable */

export class NovaClient {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Step 2: consumes the server's msg2 (from `NovaHandshakeResponse`),
     * establishes the ratcheted session, and returns msg3, base64-encoded,
     * to send as `{"type":"NovaHandshakeComplete","msg3":...}`.
     */
    completeHandshake(msg2_b64: string): string;
    /**
     * True once `completeHandshake` has succeeded — `client.js` uses this
     * to decide whether a frame should go out sealed or is still part of
     * the handshake itself.
     */
    isEstablished(): boolean;
    /**
     * A fresh, ephemeral identity — generated in the browser, held only for
     * this connection's lifetime. There is nothing to persist: TOFU, same
     * as the server's own identity (`state.rs::AppState::nova_identity`).
     */
    constructor();
    /**
     * Opens a base64 sealed record from `{"type":"Sealed","data":...}`.
     * Returns the decrypted JSON string, or `undefined` for a
     * ratchet-control record with nothing to deliver — Phase 1 never sends
     * one, so `client.js` never actually sees `undefined` here today, but
     * the type is honest about the case existing in the protocol.
     */
    open(record_b64: string): string | undefined;
    /**
     * Seals `plaintext` (already-JSON-encoded `ClientEvent`) for sending,
     * returning base64 for `{"type":"Sealed","data":...}`.
     */
    seal(plaintext: string): string;
    /**
     * Step 1: produces msg1, base64-encoded, to send as
     * `{"type":"NovaHandshakeInit","msg1":...}`.
     */
    startHandshake(): string;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_novaclient_free: (a: number, b: number) => void;
    readonly novaclient_completeHandshake: (a: number, b: number, c: number) => [number, number, number, number];
    readonly novaclient_isEstablished: (a: number) => number;
    readonly novaclient_new: () => number;
    readonly novaclient_open: (a: number, b: number, c: number) => [number, number, number, number];
    readonly novaclient_seal: (a: number, b: number, c: number) => [number, number, number, number];
    readonly novaclient_startHandshake: (a: number) => [number, number];
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
