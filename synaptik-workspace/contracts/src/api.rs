use serde::{Deserialize, Serialize};
use crate::capsule::SimCapsule;

pub type CapsId = String;
pub type PatchId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Allow,
    AllowWithPatch,
    Quarantine,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapsAnnot {
    pub verdict: Verdict,
    /// Normalized risk score (0.0..=1.0 recommended); semantics up to contracts bundle.
    pub risk: f32,
    #[serde(default)]
    pub labels: Vec<String>,
    /// Policy/bundle version used to evaluate this annotation.
    pub policy_ver: String,
    /// Optional patch plan id for ALLOW_WITH_PATCH.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch_id: Option<PatchId>,
    /// Milliseconds since Unix epoch when this annotation was produced.
    pub ts_ms: u64,
}

/// Lightweight contract trait for evaluating a capsule.
pub trait Contract {
    fn name(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn evaluate(&self, cap: &SimCapsule) -> CapsAnnot;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Replay,
    Training,
    Transfer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Denied {
    pub reason: String,
    pub verdict: Verdict,
    pub risk: f32,
    #[serde(default)]
    pub labels: Vec<String>,
}

/// Monotonic, UUIDv7-like identifier generator resilient to clock skew.
/// Guarantees strictly increasing 128-bit values across calls in a single process
/// even if system clock moves backwards. Not a full RFC 9562 implementation, but
/// preserves version/variant nibble semantics.
pub fn uuidv7() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    // We emulate an AtomicU128 using two AtomicU64 words (hi, lo) representing the last emitted value.
    // Layout: hi = milliseconds timestamp, lo = per-ms counter.
    static LAST_HI: AtomicU64 = AtomicU64::new(0); // timestamp ms
    static LAST_LO: AtomicU64 = AtomicU64::new(0); // counter for that ms

    // CAS loop to ensure monotonicity.
    loop {
        // Snapshot previous state.
        let prev_ts = LAST_HI.load(Ordering::Acquire);
        let prev_ctr = LAST_LO.load(Ordering::Acquire);

        // Current observed ms (may go backwards due to skew).
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| std::time::Duration::from_millis(0));
        let observed_ms = now.as_millis() as u64;

        // Decide next state.
        let (next_ts, next_ctr) = if observed_ms > prev_ts {
            // Fresh time tick -> reset counter.
            (observed_ms, 0u64)
        } else {
            // Clock stayed same or went backwards: stick to prev_ts and increment counter.
            (prev_ts, prev_ctr.wrapping_add(1))
        };

        // Attempt to publish: first try to update timestamp if it changed.
        if next_ts != prev_ts {
            if LAST_HI
                .compare_exchange(prev_ts, next_ts, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                // Lost race, retry.
                continue;
            }
            // Timestamp moved forward, reset counter with store (no need for CAS if we uniquely own new ts).
            LAST_LO.store(0, Ordering::Release);
        } else {
            // Same logical timestamp: increment counter with compare_exchange to avoid lost increments under contention.
            if LAST_LO
                .compare_exchange(prev_ctr, next_ctr, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                continue; // Lost race, retry.
            }
            // Guard: another thread might have advanced the HI after we read prev_ts but before we bumped LO.
            // If so, our counter increment would belong to a stale timestamp; retry to pair correctly.
            if LAST_HI.load(Ordering::Acquire) != prev_ts {
                continue;
            }
        }

        // Use the locally minted pair (now guaranteed consistent); no reload of HI to avoid skew.
        let final_ts = next_ts;
        let final_ctr = if next_ts != prev_ts { 0 } else { next_ctr };

        // Compose 128-bit value: [timestamp(64)][counter(64)]
        let x: u128 = ((final_ts as u128) << 64) | (final_ctr as u128);
        let mut bytes = x.to_be_bytes();

        // Set version (7) and variant bits.
        bytes[6] = (bytes[6] & 0x0F) | 0x70; // version
        bytes[8] = (bytes[8] & 0x3F) | 0x80; // variant

        fn hex(b: &[u8]) -> String { b.iter().map(|v| format!("{:02x}", v)).collect() }

        return format!(
            "{}-{}-{}-{}-{}",
            hex(&bytes[0..4]),
            hex(&bytes[4..6]),
            hex(&bytes[6..8]),
            hex(&bytes[8..10]),
            hex(&bytes[10..16])
        );
    }
}

