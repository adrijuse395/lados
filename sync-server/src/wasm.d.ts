// Cloudflare Workers import WebAssembly as a pre-compiled module; this tells TypeScript about it.
declare module "*.wasm" {
  const module: WebAssembly.Module;
  export default module;
}
