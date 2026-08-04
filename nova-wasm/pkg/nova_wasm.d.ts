/* tslint:disable */
/* eslint-disable */

export class NovaClient {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Establishes a session against the server's prekey bundle
     * (`NovaPreKeyBundleResponse.bundle`, base64) and returns the X3DH
     * init message, base64, to send as
     * `{"type":"NovaX3dhInit","message":...}`. Unlike the old handshake,
     * there is no further reply to wait for — `self.session` is already
     * established the moment this call returns successfully.
     */
    establishSession(bundle_b64: string): string;
    /**
     * True once `establishSession` has succeeded — `client.js` uses this
     * to decide whether a frame should go out sealed or is still part of
     * establishing the session.
     */
    isEstablished(): boolean;
    /**
     * A fresh, ephemeral signing identity and a fresh, ephemeral X3DH DH
     * identity — generated in the browser, held only for this
     * connection's lifetime. There is nothing to persist: TOFU, same as
     * the server's own keys (`state.rs::AppState::nova_dh_identity`).
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
}

/**
 * Cover-traffic decisions (`novachannel-dp`). The decision has to be made
 * here, in the browser, not server-side: it exists to hide from a
 * *network*-position observer whether this connection is sending real
 * traffic at all, and the server already sees every real send regardless
 * of what any scheduler decides.
 */
export class NovaDummyScheduler {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * True if this slot should transmit — always true when
     * `has_real_message`, otherwise true with the scheduler's calibrated
     * dummy probability. The caller (`client.js`) is responsible for
     * actually sending an indistinguishable dummy frame when this returns
     * true and there was no real message; the guarantee is about the
     * *decision bit*, and is void if a dummy is distinguishable from a
     * real send by size or timing (`novachannel-dp`'s own doc comment).
     */
    decide(has_real_message: boolean): boolean;
    /**
     * `epsilon`: the per-slot differential-privacy budget. Lower hides
     * more (higher dummy-send probability, more bandwidth); `client.js`
     * picks the actual value (`NOVA_DP_EPSILON`) — this binding is
     * mechanism, not policy.
     */
    constructor(epsilon: number);
}

/**
 * An RLN membership identity — the anonymous, rate-limited side of `nova`
 * (`session/nova_rln.rs` is the server-side verifier and nullifier set).
 * Independent of [`NovaClient`]: an anonymous post doesn't need this
 * connection's PQ-channel identity, and never carries it.
 */
export class NovaRlnIdentity {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Hex-encoded commitment for `{"type":"RlnRegister","commitment":...}`.
     */
    commitment(): string;
    /**
     * A fresh secret key, generated in the browser and never sent anywhere
     * — only its public [`commitment`](Self::commitment) and, later, proof
     * outputs ever leave this object.
     */
    constructor();
    /**
     * Proves membership + a rate-limit share for `text` at the given
     * epoch, using `path_json` (`RlnPathResponse.path`, passed through
     * verbatim as JSON text — fetched fresh immediately before this call,
     * never cached; see `session/nova_rln.rs`'s module doc for why).
     * Returns a JSON string `{"proof":...,"y":...,"nullifier":...}`, the
     * three fields `{"type":"RlnMessage",...}` needs beyond `text` itself.
     */
    prove(path_json: string, epoch: bigint, text: string): string;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_novaclient_free: (a: number, b: number) => void;
    readonly __wbg_novadummyscheduler_free: (a: number, b: number) => void;
    readonly __wbg_novarlnidentity_free: (a: number, b: number) => void;
    readonly novaclient_establishSession: (a: number, b: number, c: number) => [number, number, number, number];
    readonly novaclient_isEstablished: (a: number) => number;
    readonly novaclient_new: () => number;
    readonly novaclient_open: (a: number, b: number, c: number) => [number, number, number, number];
    readonly novaclient_seal: (a: number, b: number, c: number) => [number, number, number, number];
    readonly novadummyscheduler_decide: (a: number, b: number) => number;
    readonly novadummyscheduler_new: (a: number) => number;
    readonly novarlnidentity_commitment: (a: number) => [number, number];
    readonly novarlnidentity_new: () => number;
    readonly novarlnidentity_prove: (a: number, b: number, c: number, d: bigint, e: number, f: number) => [number, number, number, number];
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
