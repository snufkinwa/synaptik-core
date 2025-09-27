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
    config.static_memory_maximum_size(WASM_MEMORY_MAX_BYTES); // cap linear memory
    config.consume_fuel(true); // enable ATP metering (fuel)
    // Optional watchdog alternative:
    // config.epoch_interruption(true);
    let engine = Engine::new(&config)?;

    let mut linker = Linker::new(&engine);

    // No WASI imports — contract can't touch FS/network/clock
    let module = Module::new(&engine, wasm_bytes)?;
    let mut store = Store::new(&engine, ());
    // Allocate per-invocation ATP budget (fuel). Traps cleanly when exhausted.
    store.add_fuel(DEFAULT_ATP_BUDGET)?;

    // Instantiate module without giving any host functions
    let instance = linker.instantiate(&mut store, &module)?;

    // Call exported evaluate function
    let evaluate = instance
        .get_typed_func::<(i32, i32), i32>(&mut store, "evaluate")?;

    // Pass input to WASM memory
    // (In production need to handle string passing properly here)
    let ptr = 0; // assume offset 0 for this mockup
    // NOTE: Passing strings into guest memory is elided here; this is a stub.
    // The ATP and memory limits still apply to the call.
    evaluate.call(&mut store, (ptr, input.len() as i32))?;

    Ok("Evaluation complete".into())
}
