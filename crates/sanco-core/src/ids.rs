//! Row ids: UUIDv7 for new rows, UUIDv5 for rows derived from others.

use uuid::{Builder, Uuid, uuid};

use crate::rng::Rng64;

/// Namespace for derived ids whose source is not itself a UUID.
const NS: Uuid = uuid!("6f1d7a52-3c0e-4b8e-9a51-8d2f0c4e7b13");

/// Time-ordered random id.
pub fn new_v7(now_ms: i64, rng: &mut Rng64) -> String {
    Builder::from_unix_timestamp_millis(now_ms.max(0) as u64, &rng.bytes::<10>())
        .into_uuid()
        .to_string()
}

/// Random v4-style id (site ids, epochs).
pub fn new_random(rng: &mut Rng64) -> String {
    Builder::from_random_bytes(rng.bytes::<16>())
        .into_uuid()
        .to_string()
}

/// Deterministic id for a row generated from `source`, e.g.
/// `derived_id(dr_id, "stock-out")` or `derived_id(client_id, item_id)`.
/// Every PC computes the same id, so generated rows never duplicate.
pub fn derived_id(source: &str, kind: &str) -> String {
    let ns = Uuid::parse_str(source).unwrap_or_else(|_| Uuid::new_v5(&NS, source.as_bytes()));
    Uuid::new_v5(&ns, kind.as_bytes()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_ids_are_stable_and_distinct() {
        let a = derived_id("0192f0c4-0000-7000-8000-000000000001", "item-1");
        assert_eq!(
            a,
            derived_id("0192f0c4-0000-7000-8000-000000000001", "item-1")
        );
        assert_ne!(
            a,
            derived_id("0192f0c4-0000-7000-8000-000000000001", "item-2")
        );
        assert_ne!(
            derived_id("key:vat", "setting"),
            derived_id("key:fx", "setting")
        );
    }

    #[test]
    fn v7_ids_sort_by_time() {
        let mut rng = Rng64::seeded(1);
        let a = new_v7(1_000, &mut rng);
        let b = new_v7(2_000, &mut rng);
        assert!(a < b);
    }
}
