/* tslint:disable */
/* eslint-disable */

export class NovaClient {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Applies a `NovaCommit` broadcast (base64): each membership change after
     * this connection's own join, in the server's order. Errors rather than
     * panics if not yet joined or the commit doesn't fit the current epoch;
     * `client.js` logs and continues, since the legitimate case (receiving its own
     * admitting commit twice, via `NovaWelcome` and the broadcast) is harmless.
     */
    applyCommit(commit_b64: string): void;
    /**
     * Completes the `Group` join from `NovaWelcome` (`welcome`/`commit`, base64);
     * [`Self::open_group`] works from then on. `false` if already joined or if
     * they don't apply to this connection's pending key package.
     */
    completeJoin(welcome_b64: string, commit_b64: string): void;
    /**
     * This connection's view of the group epoch once joined, else
     * `undefined`. Logged next to the server's `group.epoch()` so a viewer can
     * confirm both sides agree.
     */
    epoch(): bigint | undefined;
    /**
     * Establishes the pairwise session from the server's prekey bundle
     * (`NovaPreKeyBundleResponse.bundle`, base64) and returns the base64 X3DH init
     * message for `{"type":"NovaX3dhInit","message":...}`. The session exists as
     * soon as this returns; there is no reply to wait for.
     */
    establishSession(bundle_b64: string): string;
    /**
     * True once `establishSession` succeeded; `client.js` uses it to decide
     * whether a frame goes out sealed.
     */
    isEstablished(): boolean;
    /**
     * True once [`Self::complete_join`] has succeeded.
     */
    isGroupJoined(): boolean;
    /**
     * This connection's public `LeafKeyPackage` (base64) for
     * `{"type":"NovaJoinRequest","key_package":...}`. The same package on every
     * call before joining; after `completeJoin` it errors rather than generate an
     * unrelated second leaf.
     */
    myKeyPackage(): string;
    /**
     * Fresh keys for both sessions, generated in the browser for this
     * connection's lifetime only. TOFU, like the server's own keys
     * (`AppState::nova_dh_identity`/`nova_identity`).
     */
    constructor();
    /**
     * Opens a pairwise record from `{"type":"Sealed","data":...}` (broadcasts use
     * [`Self::open_group`]). Returns the JSON, or `undefined` for a
     * ratchet-control record; the server never sends one today, but the protocol
     * allows it.
     */
    open(record_b64: string): string | undefined;
    /**
     * Opens a `{"type":"GroupSealed","data":...}` record (base64), sealed once by
     * the server (`session/nova.rs::run_seal_loop`), returning the JSON.
     */
    openGroup(record_b64: string): string;
    /**
     * Seals `plaintext` (already-JSON-encoded `ClientEvent`) for sending,
     * returning base64 for `{"type":"Sealed","data":...}`.
     */
    seal(plaintext: string): string;
}

/**
 * Cover-traffic decisions (`novachannel-dp`), made in the browser because they
 * exist to hide from a network observer whether this connection is sending;
 * the server sees every real send anyway.
 *
 * At positive epsilon this is cover traffic, not differential privacy:
 * upstream documents that a silent slot still reveals nothing was queued, and
 * that its `Budget` doesn't certify this scheduler. The `Budget` here is a cap
 * on how long the connection sends dummies: `decide` spends a slot per call,
 * real or dummy, and once it runs out dummies stop while real sends continue.
 */
export class NovaDummyScheduler {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Whether to transmit this slot: always with a real message; otherwise with
     * the scheduler's dummy probability until the cap is spent, then never. Real
     * sends are never withheld. When this says to send a dummy, `client.js` must
     * send one indistinguishable from a real frame by size and timing.
     */
    decide(has_real_message: boolean): boolean;
    /**
     * True once `remaining()` is zero and `decide` has stopped sending dummies.
     */
    isExhausted(): boolean;
    /**
     * `epsilon` sets the dummy probability (`e^-epsilon` per idle slot; lower
     * means more dummies and bandwidth). `total_budget` caps the slots spent
     * before dummies stop. `client.js` picks both (`NOVA_DP_EPSILON`,
     * `NOVA_DP_TOTAL_BUDGET`); this binding is mechanism, not policy.
     */
    constructor(epsilon: number, total_budget: number);
    /**
     * Epsilon left before cover traffic stops.
     */
    remaining(): number;
    /**
     * Total epsilon spent so far — `client.js`'s visible budget readout.
     */
    spent(): number;
}

/**
 * An RLN membership identity for anonymous, rate-limited posting
 * (`session/nova_rln.rs` verifies). Independent of [`NovaClient`]: an
 * anonymous post never carries the PQ-channel identity.
 */
export class NovaRlnIdentity {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Hex-encoded commitment for `{"type":"RlnRegister","commitment":...}`.
     */
    commitment(): string;
    /**
     * A fresh secret key that never leaves the browser; only its
     * [`commitment`](Self::commitment) and proof outputs do.
     */
    constructor();
    /**
     * Proves membership and a rate-limit share for `text` at `epoch`, using
     * `path_json` (`RlnPathResponse.path`, fetched just before, never cached; see
     * `session/nova_rln.rs`). Returns `{"proof":...,"y":...,"nullifier":...}`, the
     * fields `RlnMessage` needs besides `text`.
     */
    prove(path_json: string, epoch: bigint, text: string): string;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_novaclient_free: (a: number, b: number) => void;
    readonly __wbg_novadummyscheduler_free: (a: number, b: number) => void;
    readonly __wbg_novarlnidentity_free: (a: number, b: number) => void;
    readonly novaclient_applyCommit: (a: number, b: number, c: number) => [number, number];
    readonly novaclient_completeJoin: (a: number, b: number, c: number, d: number, e: number) => [number, number];
    readonly novaclient_epoch: (a: number) => [number, bigint];
    readonly novaclient_establishSession: (a: number, b: number, c: number) => [number, number, number, number];
    readonly novaclient_isEstablished: (a: number) => number;
    readonly novaclient_isGroupJoined: (a: number) => number;
    readonly novaclient_myKeyPackage: (a: number) => [number, number, number, number];
    readonly novaclient_new: () => number;
    readonly novaclient_open: (a: number, b: number, c: number) => [number, number, number, number];
    readonly novaclient_openGroup: (a: number, b: number, c: number) => [number, number, number, number];
    readonly novaclient_seal: (a: number, b: number, c: number) => [number, number, number, number];
    readonly novadummyscheduler_decide: (a: number, b: number) => number;
    readonly novadummyscheduler_isExhausted: (a: number) => number;
    readonly novadummyscheduler_new: (a: number, b: number) => number;
    readonly novadummyscheduler_remaining: (a: number) => number;
    readonly novadummyscheduler_spent: (a: number) => number;
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
