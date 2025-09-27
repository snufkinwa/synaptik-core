#![allow(dead_code)]
// Host-side WASM sandbox (native execution of guest contract bytecode) — EXPERIMENTAL.
// Only compiled when the `wasm` feature is enabled on this crate (see Cargo.toml).
// Not yet wired for production use; ABI and memory passing are placeholders.
//
// Roadmap:
// * Define guest ABI (allocate(len)->ptr; evaluate(ptr,len)->(ptr,len))
// * Implement safe string/buffer marshalling
// * Module validation (imports, memory limits, start fn behavior)
// * Robust error taxonomy & logging (fuel exhaustion vs contract fault)
// * Fuzzing & differential tests vs native evaluator
// * Determinism checks across platforms
//
// At present run_wasm_contract only demonstrates engine setup, fuel, and a stub call.

use wasmtime::*;

// Hardened defaults for contract sandboxing.
// - Memory cap: 64 MiB (fits guidance 16–64 MiB range)
// - ATP budget: 10M instruction-steps per invocation (guidance 5–20M)
const WASM_MEMORY_MAX_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_ATP_BUDGET: u64 = 10_000_000;

pub fn run_wasm_contract(wasm_bytes: &[u8], input: &str) -> anyhow::Result<String> {
    // Restrict WASM memory and CPU (ATP budgeting) to prevent abuse
    let mut config = Config::new();
    config.wasm_memory64(false); // forbid 64-bit linear memory
    config.static_memory_maximum_size(WASM_MEMORY_MAX_BYTES as u64); // cap linear memory
    config.consume_fuel(true); // enable ATP metering (fuel)
    // Optional watchdog alternative:
    // config.epoch_interruption(true);
    let engine = Engine::new(&config)?;

    let linker = Linker::new(&engine);

    // No WASI imports — contract can't touch FS/network/clock
    let module = Module::new(&engine, wasm_bytes)?;
    let mut store = Store::new(&engine, ());
    // Allocate per-invocation ATP budget (fuel). Traps cleanly when exhausted.
    store.add_fuel(DEFAULT_ATP_BUDGET)?;

    // Instantiate module without giving any host functions
    let instance = linker.instantiate(&mut store, &module)?;

    // Call exported evaluate function
    let evaluate = instance
        .get_func(&mut store, "evaluate")
        .ok_or_else(|| anyhow::anyhow!("missing exported function 'evaluate'"))?;

    // Attempt zero-arity first
    if let Ok(f0) = evaluate.typed::<(), ()>(&store) {
        f0.call(&mut store, ())?;
        return Ok("wasm_sandbox_stub".into());
    }
    // Attempt (i32,i32)->i32 legacy style
    if let Ok(f_legacy) = evaluate.typed::<(i32, i32), i32>(&store) {
        let _ = f_legacy.call(&mut store, (0, input.len() as i32))?;
        return Ok("wasm_sandbox_stub".into());
    }
    Err(anyhow::anyhow!(
        "unsupported 'evaluate' signature (expected () or (i32,i32)->i32)"
    ))
}
