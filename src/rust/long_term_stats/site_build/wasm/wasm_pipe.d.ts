/* tslint:disable */
/* eslint-disable */
/**
* @param {string} url
*/
export function connect_wasm_pipe(url: string): void;
/**
* @returns {boolean}
*/
export function is_wasm_connected(): boolean;
/**
*/
export function send_wss_queue(): void;
/**
* @param {string} token
*/
export function send_token(token: string): void;
/**
* @param {string} license
* @param {string} username
* @param {string} password
*/
export function send_login(license: string, username: string, password: string): void;
/**
*/
export function request_node_status(): void;
/**
* @param {string} period
*/
export function request_packet_chart(period: string): void;
/**
* @param {string} period
* @param {string} node_id
* @param {string} node_name
*/
export function request_packet_chart_for_node(period: string, node_id: string, node_name: string): void;
/**
* @param {string} period
*/
export function request_throughput_chart(period: string): void;
/**
* @param {string} period
* @param {string} site_id
*/
export function request_throughput_chart_for_site(period: string, site_id: string): void;
/**
* @param {string} period
* @param {string} node_id
* @param {string} node_name
*/
export function request_throughput_chart_for_node(period: string, node_id: string, node_name: string): void;
/**
* @param {string} period
* @param {string} circuit_id
*/
export function request_throughput_chart_for_circuit(period: string, circuit_id: string): void;
/**
* @param {string} period
* @param {string} site_id
*/
export function request_site_stack(period: string, site_id: string): void;
/**
* @param {string} period
*/
export function request_rtt_chart(period: string): void;
/**
* @param {string} period
* @param {string} site_id
*/
export function request_rtt_chart_for_site(period: string, site_id: string): void;
/**
* @param {string} period
* @param {string} node_id
* @param {string} node_name
*/
export function request_rtt_chart_for_node(period: string, node_id: string, node_name: string): void;
/**
* @param {string} period
* @param {string} circuit_id
*/
export function request_rtt_chart_for_circuit(period: string, circuit_id: string): void;
/**
* @param {string} period
* @param {string} node_id
* @param {string} node_name
*/
export function request_node_perf_chart(period: string, node_id: string, node_name: string): void;
/**
* @param {string} period
*/
export function request_root_heat(period: string): void;
/**
* @param {string} period
* @param {string} site_id
*/
export function request_site_heat(period: string, site_id: string): void;
/**
* @param {string} parent
*/
export function request_tree(parent: string): void;
/**
* @param {string} site_id
*/
export function request_site_info(site_id: string): void;
/**
* @param {string} site_id
*/
export function request_site_parents(site_id: string): void;
/**
* @param {string} circuit_id
*/
export function request_circuit_parents(circuit_id: string): void;
/**
*/
export function request_root_parents(): void;
/**
* @param {string} term
*/
export function request_search(term: string): void;
/**
* @param {string} circuit_id
*/
export function request_circuit_info(circuit_id: string): void;
/**
* @param {string} circuit_id
*/
export function request_ext_device_info(circuit_id: string): void;
/**
* @param {string} period
* @param {string} device_id
*/
export function request_ext_snr_graph(period: string, device_id: string): void;
/**
* @param {string} period
* @param {string} device_id
*/
export function request_ext_capacity_graph(period: string, device_id: string): void;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
  readonly memory: WebAssembly.Memory;
  readonly connect_wasm_pipe: (a: number, b: number) => void;
  readonly is_wasm_connected: () => number;
  readonly send_wss_queue: () => void;
  readonly send_token: (a: number, b: number) => void;
  readonly send_login: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
  readonly request_node_status: () => void;
  readonly request_packet_chart: (a: number, b: number) => void;
  readonly request_packet_chart_for_node: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
  readonly request_throughput_chart: (a: number, b: number) => void;
  readonly request_throughput_chart_for_site: (a: number, b: number, c: number, d: number) => void;
  readonly request_throughput_chart_for_node: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
  readonly request_throughput_chart_for_circuit: (a: number, b: number, c: number, d: number) => void;
  readonly request_site_stack: (a: number, b: number, c: number, d: number) => void;
  readonly request_rtt_chart: (a: number, b: number) => void;
  readonly request_rtt_chart_for_site: (a: number, b: number, c: number, d: number) => void;
  readonly request_rtt_chart_for_node: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
  readonly request_rtt_chart_for_circuit: (a: number, b: number, c: number, d: number) => void;
  readonly request_node_perf_chart: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
  readonly request_root_heat: (a: number, b: number) => void;
  readonly request_site_heat: (a: number, b: number, c: number, d: number) => void;
  readonly request_tree: (a: number, b: number) => void;
  readonly request_site_info: (a: number, b: number) => void;
  readonly request_site_parents: (a: number, b: number) => void;
  readonly request_circuit_parents: (a: number, b: number) => void;
  readonly request_root_parents: () => void;
  readonly request_search: (a: number, b: number) => void;
  readonly request_circuit_info: (a: number, b: number) => void;
  readonly request_ext_device_info: (a: number, b: number) => void;
  readonly request_ext_snr_graph: (a: number, b: number, c: number, d: number) => void;
  readonly request_ext_capacity_graph: (a: number, b: number, c: number, d: number) => void;
  readonly __wbindgen_malloc: (a: number) => number;
  readonly __wbindgen_realloc: (a: number, b: number, c: number) => number;
  readonly __wbindgen_export_2: WebAssembly.Table;
  readonly _dyn_core__ops__function__FnMut__A____Output___R_as_wasm_bindgen__closure__WasmClosure___describe__invoke__hb4b341652e081e3f: (a: number, b: number, c: number) => void;
  readonly _dyn_core__ops__function__FnMut__A____Output___R_as_wasm_bindgen__closure__WasmClosure___describe__invoke__ha318d2d73313995c: (a: number, b: number, c: number) => void;
  readonly __wbindgen_free: (a: number, b: number) => void;
  readonly __wbindgen_exn_store: (a: number) => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;
/**
* Instantiates the given `module`, which can either be bytes or
* a precompiled `WebAssembly.Module`.
*
* @param {SyncInitInput} module
*
* @returns {InitOutput}
*/
export function initSync(module: SyncInitInput): InitOutput;

/**
* If `module_or_path` is {RequestInfo} or {URL}, makes a request and
* for everything else, calls `WebAssembly.instantiate` directly.
*
* @param {InitInput | Promise<InitInput>} module_or_path
*
* @returns {Promise<InitOutput>}
*/
export default function __wbg_init (module_or_path?: InitInput | Promise<InitInput>): Promise<InitOutput>;
