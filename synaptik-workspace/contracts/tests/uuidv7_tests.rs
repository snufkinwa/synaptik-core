use std::collections::HashSet;

use contracts::api::uuidv7;

// Parse the UUIDv7-like string into a u128 for ordering checks.
fn parse_uuid_u128(id: &str) -> u128 {
    let hex: String = id.chars().filter(|c| *c != '-').collect();
    assert_eq!(hex.len(), 32, "uuid should have 32 hex chars (got {})", hex.len());
    u128::from_str_radix(&hex, 16).expect("failed to parse hex uuid into u128")
}

#[test]
fn test_uuidv7_monotonic_and_unique() {
    // Generate a reasonably sized sequence to exercise counter rollover within same ms.
    // (If the clock does not advance quickly enough, counter increments keep monotonic order.)
    const N: usize = 5_000;
    let mut ids: Vec<String> = Vec::with_capacity(N);
    for _ in 0..N { ids.push(uuidv7()); }

    // Basic formatting & version / variant checks while collecting numeric forms.
    let mut numbers = Vec::with_capacity(N);
    let mut seen = HashSet::with_capacity(N);
    for id in &ids {
        // Format groups: 8-4-4-4-12
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.len(), 5, "uuid format groups");
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);

        // Version nibble (first char of 3rd group) should be '7'.
        assert_eq!(parts[2].chars().next().unwrap(), '7', "version nibble should be 7");
        // Variant nibble (first char of 4th group) should be one of 8,9,a,b (RFC 4122 variant 1).
        let variant_ch = parts[3].chars().next().unwrap();
        assert!(matches!(variant_ch, '8' | '9' | 'a' | 'b'), "variant nibble invalid: {}", variant_ch);

        let num = parse_uuid_u128(id);
        numbers.push(num);
        assert!(seen.insert(id), "duplicate uuid encountered: {id}");
    }

    // Strictly increasing numeric order.
    for w in numbers.windows(2) {
        assert!(w[0] < w[1], "uuids not strictly increasing: {} !< {}", w[0], w[1]);
    }

    assert_eq!(seen.len(), N, "all ids must be unique");
}

#[test]
fn test_uuidv7_concurrency_uniqueness() {
    use std::thread;
    use std::sync::{Arc, Barrier, Mutex};

    const THREADS: usize = 8;
    const PER_THREAD: usize = 800; // 8 * 800 = 6400 ids

    let barrier = Arc::new(Barrier::new(THREADS));
    let all: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::with_capacity(THREADS * PER_THREAD)));

    let mut handles = Vec::with_capacity(THREADS);
    for _ in 0..THREADS {
        let b = barrier.clone();
        let out = all.clone();
        handles.push(thread::spawn(move || {
            // Synchronize start to maximize contention.
            b.wait();
            let mut local = Vec::with_capacity(PER_THREAD);
            for _ in 0..PER_THREAD { local.push(uuidv7()); }
            out.lock().unwrap().extend(local);
        }));
    }
    for h in handles { h.join().expect("thread panicked"); }

    let guard = all.lock().unwrap();
    let mut set = HashSet::with_capacity(guard.len());
    for id in guard.iter() {
        assert!(set.insert(id), "duplicate id under concurrency: {id}");
        // Spot check version & variant again.
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts[2].chars().next().unwrap(), '7');
        let variant_ch = parts[3].chars().next().unwrap();
        assert!(matches!(variant_ch, '8' | '9' | 'a' | 'b'));
    }
    assert_eq!(set.len(), THREADS * PER_THREAD);

    // We don't assert global ordering here because cross-thread completion order is non-deterministic.
}
