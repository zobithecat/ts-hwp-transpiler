// Thin TS front-end. The real work is in crates/wasm, surfaced via wasm-pack.
// Run `pnpm build:wasm` (or npm/yarn equivalent) to regenerate ./src/wasm.

import init, { version, tokenizeFormula } from "./wasm/hwp_transpiler_wasm.js";

export interface Hint {
  kind: "char" | "para";
  [key: string]: unknown;
}

export interface TranspileResult {
  markdown: string;
  hints: Hint[];
}

export async function ready(): Promise<string> {
  await init();
  return version();
}

export async function tokenize(script: string): Promise<string[]> {
  await init();
  return tokenizeFormula(script);
}
