use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncDirection {
    TwoWay,
    LeftToRight,
    RightToLeft,
}

impl Default for SyncDirection {
    fn default() -> Self {
        Self::TwoWay
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    Manual,
    GoogleWins,
    IcloudWins,
    NewestUpdatedWins,
}

impl Default for ConflictPolicy {
    fn default() -> Self {
        Self::Manual
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteConflictPolicy {
    Manual,
    DeleteWins,
    UpdateWins,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UidCollisionPolicy {
    Manual,
    IgnoreKnown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictPolicies {
    pub default: ConflictPolicy,
    pub both_sides_changed: ConflictPolicy,
    pub unlinked_same_uid: ConflictPolicy,
    pub delete_vs_update: DeleteConflictPolicy,
    pub icloud_uid_collision: UidCollisionPolicy,
}

impl Default for ConflictPolicies {
    fn default() -> Self {
        Self {
            default: ConflictPolicy::Manual,
            both_sides_changed: ConflictPolicy::Manual,
            unlinked_same_uid: ConflictPolicy::Manual,
            delete_vs_update: DeleteConflictPolicy::UpdateWins,
            icloud_uid_collision: UidCollisionPolicy::IgnoreKnown,
        }
    }
}
