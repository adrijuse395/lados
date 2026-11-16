/* tslint:disable */
/* eslint-disable */

/**
 * The server's OPRF response to a client registration request (base64).
 */
export function create_registration_response(server_setup: string, user_identifier: string, registration_request: string): string;

/**
 * Finishes a login, returning `{ "sessionKey": ... }` as JSON.
 */
export function finish_login(server_login_state: string, finish_login_request: string): string;

/**
 * Starts a login. `seed` is 32 bytes of caller-supplied randomness. Returns
 * `{ "serverLoginState", "loginResponse" }` as JSON.
 */
export function start_login(server_setup: string, registration_record: string | null | undefined, start_login_request: string, user_identifier: string, seed: Uint8Array | number[]): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly create_registration_response: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number, number];
    readonly finish_login: (a: number, b: number, c: number, d: number) => [number, number, number, number];
    readonly start_login: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => [number, number, number, number];
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __externref_table_dealloc: (a: number) => void;
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
