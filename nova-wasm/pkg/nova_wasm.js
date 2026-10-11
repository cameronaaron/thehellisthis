/* @ts-self-types="./nova_wasm.d.ts" */

export class NovaClient {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        NovaClientFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_novaclient_free(ptr, 0);
    }
    /**
     * Applies a `NovaCommit` broadcast (base64): each membership change after
     * this connection's own join, in the server's order. Errors rather than
     * panics if not yet joined or the commit doesn't fit the current epoch;
     * `client.js` logs and continues, since the legitimate case (receiving its own
     * admitting commit twice, via `NovaWelcome` and the broadcast) is harmless.
     * @param {string} commit_b64
     */
    applyCommit(commit_b64) {
        const ptr0 = passStringToWasm0(commit_b64, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.novaclient_applyCommit(this.__wbg_ptr, ptr0, len0);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Completes the `Group` join from `NovaWelcome` (`welcome`/`commit`, base64);
     * [`Self::open_group`] works from then on. `false` if already joined or if
     * they don't apply to this connection's pending key package.
     * @param {string} welcome_b64
     * @param {string} commit_b64
     */
    completeJoin(welcome_b64, commit_b64) {
        const ptr0 = passStringToWasm0(welcome_b64, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passStringToWasm0(commit_b64, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        const ret = wasm.novaclient_completeJoin(this.__wbg_ptr, ptr0, len0, ptr1, len1);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * This connection's view of the group epoch once joined, else
     * `undefined`. Logged next to the server's `group.epoch()` so a viewer can
     * confirm both sides agree.
     * @returns {bigint | undefined}
     */
    epoch() {
        const ret = wasm.novaclient_epoch(this.__wbg_ptr);
        return ret[0] === 0 ? undefined : BigInt.asUintN(64, ret[1]);
    }
    /**
     * Establishes the pairwise session from the server's prekey bundle
     * (`NovaPreKeyBundleResponse.bundle`, base64) and returns the base64 X3DH init
     * message for `{"type":"NovaX3dhInit","message":...}`. The session exists as
     * soon as this returns; there is no reply to wait for.
     * @param {string} bundle_b64
     * @returns {string}
     */
    establishSession(bundle_b64) {
        let deferred3_0;
        let deferred3_1;
        try {
            const ptr0 = passStringToWasm0(bundle_b64, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len0 = WASM_VECTOR_LEN;
            const ret = wasm.novaclient_establishSession(this.__wbg_ptr, ptr0, len0);
            var ptr2 = ret[0];
            var len2 = ret[1];
            if (ret[3]) {
                ptr2 = 0; len2 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred3_0 = ptr2;
            deferred3_1 = len2;
            return getStringFromWasm0(ptr2, len2);
        } finally {
            wasm.__wbindgen_free(deferred3_0, deferred3_1, 1);
        }
    }
    /**
     * True once `establishSession` succeeded; `client.js` uses it to decide
     * whether a frame goes out sealed.
     * @returns {boolean}
     */
    isEstablished() {
        const ret = wasm.novaclient_isEstablished(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * True once [`Self::complete_join`] has succeeded.
     * @returns {boolean}
     */
    isGroupJoined() {
        const ret = wasm.novaclient_isGroupJoined(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * This connection's public `LeafKeyPackage` (base64) for
     * `{"type":"NovaJoinRequest","key_package":...}`. The same package on every
     * call before joining; after `completeJoin` it errors rather than generate an
     * unrelated second leaf.
     * @returns {string}
     */
    myKeyPackage() {
        let deferred2_0;
        let deferred2_1;
        try {
            const ret = wasm.novaclient_myKeyPackage(this.__wbg_ptr);
            var ptr1 = ret[0];
            var len1 = ret[1];
            if (ret[3]) {
                ptr1 = 0; len1 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred2_0 = ptr1;
            deferred2_1 = len1;
            return getStringFromWasm0(ptr1, len1);
        } finally {
            wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
        }
    }
    /**
     * Fresh keys for both sessions, generated in the browser for this
     * connection's lifetime only. TOFU, like the server's own keys
     * (`AppState::nova_dh_identity`/`nova_identity`).
     */
    constructor() {
        const ret = wasm.novaclient_new();
        this.__wbg_ptr = ret;
        NovaClientFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * Opens a pairwise record from `{"type":"Sealed","data":...}` (broadcasts use
     * [`Self::open_group`]). Returns the JSON, or `undefined` for a
     * ratchet-control record; the server never sends one today, but the protocol
     * allows it.
     * @param {string} record_b64
     * @returns {string | undefined}
     */
    open(record_b64) {
        const ptr0 = passStringToWasm0(record_b64, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.novaclient_open(this.__wbg_ptr, ptr0, len0);
        if (ret[3]) {
            throw takeFromExternrefTable0(ret[2]);
        }
        let v2;
        if (ret[0] !== 0) {
            v2 = getStringFromWasm0(ret[0], ret[1]).slice();
            wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        }
        return v2;
    }
    /**
     * Opens a `{"type":"GroupSealed","data":...}` record (base64), sealed once by
     * the server (`session/nova.rs::run_seal_loop`), returning the JSON.
     * @param {string} record_b64
     * @returns {string}
     */
    openGroup(record_b64) {
        let deferred3_0;
        let deferred3_1;
        try {
            const ptr0 = passStringToWasm0(record_b64, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len0 = WASM_VECTOR_LEN;
            const ret = wasm.novaclient_openGroup(this.__wbg_ptr, ptr0, len0);
            var ptr2 = ret[0];
            var len2 = ret[1];
            if (ret[3]) {
                ptr2 = 0; len2 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred3_0 = ptr2;
            deferred3_1 = len2;
            return getStringFromWasm0(ptr2, len2);
        } finally {
            wasm.__wbindgen_free(deferred3_0, deferred3_1, 1);
        }
    }
    /**
     * Seals `plaintext` (already-JSON-encoded `ClientEvent`) for sending,
     * returning base64 for `{"type":"Sealed","data":...}`.
     * @param {string} plaintext
     * @returns {string}
     */
    seal(plaintext) {
        let deferred3_0;
        let deferred3_1;
        try {
            const ptr0 = passStringToWasm0(plaintext, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len0 = WASM_VECTOR_LEN;
            const ret = wasm.novaclient_seal(this.__wbg_ptr, ptr0, len0);
            var ptr2 = ret[0];
            var len2 = ret[1];
            if (ret[3]) {
                ptr2 = 0; len2 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred3_0 = ptr2;
            deferred3_1 = len2;
            return getStringFromWasm0(ptr2, len2);
        } finally {
            wasm.__wbindgen_free(deferred3_0, deferred3_1, 1);
        }
    }
}
if (Symbol.dispose) NovaClient.prototype[Symbol.dispose] = NovaClient.prototype.free;

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
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        NovaDummySchedulerFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_novadummyscheduler_free(ptr, 0);
    }
    /**
     * Whether to transmit this slot: always with a real message; otherwise with
     * the scheduler's dummy probability until the cap is spent, then never. Real
     * sends are never withheld. When this says to send a dummy, `client.js` must
     * send one indistinguishable from a real frame by size and timing.
     * @param {boolean} has_real_message
     * @returns {boolean}
     */
    decide(has_real_message) {
        const ret = wasm.novadummyscheduler_decide(this.__wbg_ptr, has_real_message);
        return ret !== 0;
    }
    /**
     * True once `remaining()` is zero and `decide` has stopped sending dummies.
     * @returns {boolean}
     */
    isExhausted() {
        const ret = wasm.novadummyscheduler_isExhausted(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * `epsilon` sets the dummy probability (`e^-epsilon` per idle slot; lower
     * means more dummies and bandwidth). `total_budget` caps the slots spent
     * before dummies stop. `client.js` picks both (`NOVA_DP_EPSILON`,
     * `NOVA_DP_TOTAL_BUDGET`); this binding is mechanism, not policy.
     * @param {number} epsilon
     * @param {number} total_budget
     */
    constructor(epsilon, total_budget) {
        const ret = wasm.novadummyscheduler_new(epsilon, total_budget);
        this.__wbg_ptr = ret;
        NovaDummySchedulerFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * Epsilon left before cover traffic stops.
     * @returns {number}
     */
    remaining() {
        const ret = wasm.novadummyscheduler_remaining(this.__wbg_ptr);
        return ret;
    }
    /**
     * Total epsilon spent so far — `client.js`'s visible budget readout.
     * @returns {number}
     */
    spent() {
        const ret = wasm.novadummyscheduler_spent(this.__wbg_ptr);
        return ret;
    }
}
if (Symbol.dispose) NovaDummyScheduler.prototype[Symbol.dispose] = NovaDummyScheduler.prototype.free;

/**
 * An RLN membership identity for anonymous, rate-limited posting
 * (`session/nova_rln.rs` verifies). Independent of [`NovaClient`]: an
 * anonymous post never carries the PQ-channel identity.
 */
export class NovaRlnIdentity {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        NovaRlnIdentityFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_novarlnidentity_free(ptr, 0);
    }
    /**
     * Hex-encoded commitment for `{"type":"RlnRegister","commitment":...}`.
     * @returns {string}
     */
    commitment() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.novarlnidentity_commitment(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * A fresh secret key that never leaves the browser; only its
     * [`commitment`](Self::commitment) and proof outputs do.
     */
    constructor() {
        const ret = wasm.novarlnidentity_new();
        this.__wbg_ptr = ret;
        NovaRlnIdentityFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * Proves membership and a rate-limit share for `text` at `epoch`, using
     * `path_json` (`RlnPathResponse.path`, fetched just before, never cached; see
     * `session/nova_rln.rs`). Returns `{"proof":...,"y":...,"nullifier":...}`, the
     * fields `RlnMessage` needs besides `text`.
     * @param {string} path_json
     * @param {bigint} epoch
     * @param {string} text
     * @returns {string}
     */
    prove(path_json, epoch, text) {
        let deferred4_0;
        let deferred4_1;
        try {
            const ptr0 = passStringToWasm0(path_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(text, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            const ret = wasm.novarlnidentity_prove(this.__wbg_ptr, ptr0, len0, epoch, ptr1, len1);
            var ptr3 = ret[0];
            var len3 = ret[1];
            if (ret[3]) {
                ptr3 = 0; len3 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred4_0 = ptr3;
            deferred4_1 = len3;
            return getStringFromWasm0(ptr3, len3);
        } finally {
            wasm.__wbindgen_free(deferred4_0, deferred4_1, 1);
        }
    }
}
if (Symbol.dispose) NovaRlnIdentity.prototype[Symbol.dispose] = NovaRlnIdentity.prototype.free;
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg___wbindgen_throw_344f42d3211c4765: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbg_getRandomValues_cc7f052a444bb2ce: function() { return handleError(function (arg0, arg1) {
            globalThis.crypto.getRandomValues(getArrayU8FromWasm0(arg0, arg1));
        }, arguments); },
        __wbindgen_cast_0000000000000001: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_init_externref_table: function() {
            const table = wasm.__wbindgen_externrefs;
            const offset = table.grow(4);
            table.set(0, undefined);
            table.set(offset + 0, undefined);
            table.set(offset + 1, null);
            table.set(offset + 2, true);
            table.set(offset + 3, false);
        },
    };
    return {
        __proto__: null,
        "./nova_wasm_bg.js": import0,
    };
}

const NovaClientFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_novaclient_free(ptr, 1));
const NovaDummySchedulerFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_novadummyscheduler_free(ptr, 1));
const NovaRlnIdentityFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_novarlnidentity_free(ptr, 1));

function addToExternrefTable0(obj) {
    const idx = wasm.__externref_table_alloc();
    wasm.__wbindgen_externrefs.set(idx, obj);
    return idx;
}

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function handleError(f, args) {
    try {
        return f.apply(this, args);
    } catch (e) {
        const idx = addToExternrefTable0(e);
        wasm.__wbindgen_exn_store(idx);
    }
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

function takeFromExternrefTable0(idx) {
    const value = wasm.__wbindgen_externrefs.get(idx);
    wasm.__externref_table_dealloc(idx);
    return value;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasmInstance, wasm;
function __wbg_finalize_init(instance, module) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module;
    cachedUint8ArrayMemory0 = null;
    wasm.__wbindgen_start();
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = module.ok && expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        module_or_path = new URL('nova_wasm_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };
