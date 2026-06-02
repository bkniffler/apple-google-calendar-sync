pub mod configured_pairs;
pub mod event_links;
pub mod sync_conflicts;
pub mod sync_runs;
pub mod sync_state;

use sha2::{Digest, Sha256};

pub fn stable_id(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(parts.join("\0").as_bytes());
    let digest = hasher.finalize();
    format!("{digest:x}").chars().take(24).collect()
}
