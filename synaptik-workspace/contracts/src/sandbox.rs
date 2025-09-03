use wasmtime::*;

pub fn run_wasm_contract(wasm_bytes: &[u8], input: &str) -> anyhow::Result<String> {
    let engine = Engine::default();

    // Restrict WASM memory to prevent abuse
    let mut config = Config::new();
    config.wasm_memory64(false); // no 64-bit memory
    config.static_memory_maximum_size(1024 * 64); // 64KB max
    let engine = Engine::new(&config)?;

    let mut linker = Linker::new(&engine);

    // No WASI imports — contract can't touch FS/network/clock
    let module = Module::new(&engine, wasm_bytes)?;
    let mut store = Store::new(&engine, ());

    // Instantiate module without giving any host functions
    let instance = linker.instantiate(&mut store, &module)?;

    // Call exported evaluate function
    let evaluate = instance
        .get_typed_func::<(i32, i32), i32>(&mut store, "evaluate")?;

    // Pass input to WASM memory
    // (In production need to handle string passing properly here)
    let ptr = 0; // assume offset 0 for this mockup
    evaluate.call(&mut store, (ptr, input.len() as i32))?;

    Ok("Evaluation complete".into())
}
