use crate::{
    CanonicalEvent, ConflictPolicy, DeleteConflictPolicy, SyncDirection, UidCollisionPolicy,
    hash_canonical_event,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EventLink {
    pub id: String,
    pub sync_pair_id: String,
    pub canonical_uid: String,
    pub google_event_id: Option<String>,
    pub google_ical_uid: Option<String>,
    pub google_etag: Option<String>,
    pub icloud_href: Option<String>,
    pub icloud_uid: Option<String>,
    pub icloud_etag: Option<String>,
    pub google_hash: Option<String>,
    pub icloud_hash: Option<String>,
    pub last_synced_hash: Option<String>,
    pub deleted_google_at: Option<String>,
    pub deleted_icloud_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerConflictPolicies {
    pub both_sides_changed: ConflictPolicy,
    pub unlinked_same_uid: ConflictPolicy,
    pub delete_vs_update: DeleteConflictPolicy,
    pub icloud_uid_collision: UidCollisionPolicy,
}

impl Default for PlannerConflictPolicies {
    fn default() -> Self {
        Self {
            both_sides_changed: ConflictPolicy::Manual,
            unlinked_same_uid: ConflictPolicy::Manual,
            delete_vs_update: DeleteConflictPolicy::UpdateWins,
            icloud_uid_collision: UidCollisionPolicy::IgnoreKnown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoResolution {
    pub reason: String,
    pub policy: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlannedAction {
    Snapshot {
        canonical_uid: String,
        link: Option<EventLink>,
        google: Option<Box<CanonicalEvent>>,
        icloud: Option<Box<CanonicalEvent>>,
        google_hash: Option<String>,
        icloud_hash: Option<String>,
        resolution: Option<AutoResolution>,
    },
    Noop {
        canonical_uid: String,
        reason: String,
    },
    CreateGoogle(MutatingAction),
    CreateIcloud(MutatingAction),
    UpdateGoogle(MutatingAction),
    UpdateIcloud(MutatingAction),
    DeleteGoogle(MutatingAction),
    DeleteIcloud(MutatingAction),
    Conflict {
        canonical_uid: String,
        reason: String,
        resolution: ConflictResolution,
        link: Option<EventLink>,
        google: Option<Box<CanonicalEvent>>,
        icloud: Option<Box<CanonicalEvent>>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MutatingAction {
    pub canonical_uid: String,
    pub event: Box<CanonicalEvent>,
    pub link: Option<EventLink>,
    pub google: Option<Box<CanonicalEvent>>,
    pub icloud: Option<Box<CanonicalEvent>>,
    pub google_hash: Option<String>,
    pub icloud_hash: Option<String>,
    pub resolution: Option<AutoResolution>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolution {
    Manual,
    Ignored,
}

#[derive(Debug, Clone)]
pub struct PlanTwoWayInput<'a> {
    pub links: &'a [EventLink],
    pub google_events: &'a [CanonicalEvent],
    pub icloud_events: &'a [CanonicalEvent],
    pub known_icloud_uid_collisions: HashSet<String>,
    pub direction: SyncDirection,
    pub conflict_policy: PlannerConflictPolicies,
}

pub fn plan_initial_actions(
    google_events: &[CanonicalEvent],
    icloud_events: &[CanonicalEvent],
) -> Vec<PlannedAction> {
    let google_by_uid = current_events_by_uid(google_events);
    let icloud_by_uid = current_events_by_uid(icloud_events);
    let mut uids = Vec::new();
    let mut seen = HashSet::new();
    append_uids(&mut uids, &mut seen, google_by_uid.keys());
    append_uids(&mut uids, &mut seen, icloud_by_uid.keys());

    uids.into_iter()
        .map(|uid| {
            let google = google_by_uid.get(&uid);
            let icloud = icloud_by_uid.get(&uid);
            match (google, icloud) {
                (Some(google), None) => create_icloud(&uid, google, None, Some(google), None, None),
                (None, Some(icloud)) => create_google(&uid, icloud, None, None, Some(icloud), None),
                _ => PlannedAction::Noop {
                    canonical_uid: uid,
                    reason: "present_on_both_sides".to_string(),
                },
            }
        })
        .collect()
}

pub fn plan_two_way_actions(input: PlanTwoWayInput<'_>) -> Vec<PlannedAction> {
    let google_by_uid = current_events_by_uid(input.google_events);
    let icloud_by_uid = current_events_by_uid(input.icloud_events);
    let links_by_uid = input
        .links
        .iter()
        .map(|link| (link.canonical_uid.clone(), link.clone()))
        .collect::<HashMap<_, _>>();

    let mut uids = Vec::new();
    let mut seen = HashSet::new();
    append_uids(&mut uids, &mut seen, google_by_uid.keys());
    append_uids(&mut uids, &mut seen, icloud_by_uid.keys());
    append_uids(&mut uids, &mut seen, links_by_uid.keys());

    let mut actions = Vec::new();

    for uid in uids {
        let link = links_by_uid.get(&uid);
        let google = google_by_uid.get(&uid).copied();
        let icloud = icloud_by_uid.get(&uid).copied();
        let google_hash = google.map(hash_canonical_event);
        let icloud_hash = icloud.map(hash_canonical_event);
        let google_changed = google_hash != link.and_then(|item| item.google_hash.clone());
        let icloud_changed = icloud_hash != link.and_then(|item| item.icloud_hash.clone());
        let google_deleted = google.is_none()
            && link.is_some_and(|item| {
                item.google_event_id.is_some() || item.google_ical_uid.is_some()
            });
        let icloud_deleted = icloud.is_none()
            && link.is_some_and(|item| item.icloud_href.is_some() || item.icloud_uid.is_some());

        if input.known_icloud_uid_collisions.contains(&uid) && google.is_some() && icloud.is_none()
        {
            actions.push(PlannedAction::Conflict {
                canonical_uid: uid,
                reason: "icloud_uid_exists_in_different_calendar".to_string(),
                resolution: match input.conflict_policy.icloud_uid_collision {
                    UidCollisionPolicy::IgnoreKnown => ConflictResolution::Ignored,
                    UidCollisionPolicy::Manual => ConflictResolution::Manual,
                },
                link: link.cloned(),
                google: google.cloned().map(Box::new),
                icloud: None,
            });
            continue;
        }

        let Some(link) = link else {
            actions.extend(plan_unlinked(
                &uid,
                google,
                icloud,
                input.direction,
                input.conflict_policy.unlinked_same_uid,
            ));
            continue;
        };

        if let (Some(google), Some(icloud)) = (google, icloud) {
            if google_hash == icloud_hash {
                actions.push(snapshot(
                    &uid,
                    Some(link),
                    Some(google),
                    Some(icloud),
                    google_hash,
                    icloud_hash,
                ));
            } else if google_changed && !icloud_changed && can_write_icloud(input.direction) {
                actions.push(update_icloud(
                    &uid,
                    google,
                    Some(link),
                    Some(google),
                    Some(icloud),
                    google_hash,
                    icloud_hash,
                    None,
                ));
            } else if !google_changed && icloud_changed && can_write_google(input.direction) {
                actions.push(update_google(
                    &uid,
                    icloud,
                    Some(link),
                    Some(google),
                    Some(icloud),
                    google_hash,
                    icloud_hash,
                    None,
                ));
            } else if google_changed && icloud_changed {
                actions.push(resolve_provider_winner_conflict(ProviderWinnerInput {
                    uid: &uid,
                    reason: "both_sides_changed",
                    link: Some(link),
                    google,
                    icloud,
                    policy: input.conflict_policy.both_sides_changed,
                    direction: input.direction,
                }));
            } else {
                actions.push(snapshot(
                    &uid,
                    Some(link),
                    Some(google),
                    Some(icloud),
                    google_hash,
                    icloud_hash,
                ));
            }
            continue;
        }

        if let (Some(google), true) = (google, icloud_deleted) {
            if google_changed {
                actions.push(resolve_delete_update_conflict(DeleteUpdateInput {
                    uid: &uid,
                    reason: "google_changed_while_icloud_deleted",
                    deleted_side: DeletedSide::Icloud,
                    changed_event: google,
                    link: Some(link),
                    direction: input.direction,
                    policy: input.conflict_policy.delete_vs_update,
                }));
            } else if can_write_google(input.direction) {
                actions.push(delete_google(
                    &uid,
                    google,
                    Some(link),
                    Some(google),
                    None,
                    google_hash,
                ));
            } else if can_write_icloud(input.direction) {
                actions.push(create_icloud(
                    &uid,
                    google,
                    Some(link),
                    Some(google),
                    None,
                    google_hash,
                ));
            }
            continue;
        }

        if let (Some(icloud), true) = (icloud, google_deleted) {
            if icloud_changed {
                actions.push(resolve_delete_update_conflict(DeleteUpdateInput {
                    uid: &uid,
                    reason: "icloud_changed_while_google_deleted",
                    deleted_side: DeletedSide::Google,
                    changed_event: icloud,
                    link: Some(link),
                    direction: input.direction,
                    policy: input.conflict_policy.delete_vs_update,
                }));
            } else if can_write_icloud(input.direction) {
                actions.push(delete_icloud(
                    &uid,
                    icloud,
                    Some(link),
                    None,
                    Some(icloud),
                    icloud_hash,
                ));
            } else if can_write_google(input.direction) {
                actions.push(create_google(
                    &uid,
                    icloud,
                    Some(link),
                    None,
                    Some(icloud),
                    icloud_hash,
                ));
            }
            continue;
        }

        if let Some(google) = google
            && can_write_icloud(input.direction)
        {
            actions.push(create_icloud(
                &uid,
                google,
                Some(link),
                Some(google),
                None,
                google_hash,
            ));
            continue;
        }

        if let Some(icloud) = icloud
            && can_write_google(input.direction)
        {
            actions.push(create_google(
                &uid,
                icloud,
                Some(link),
                None,
                Some(icloud),
                icloud_hash,
            ));
            continue;
        }

        actions.push(snapshot(
            &uid,
            Some(link),
            google,
            icloud,
            google_hash,
            icloud_hash,
        ));
    }

    actions
}

fn plan_unlinked(
    uid: &str,
    google: Option<&CanonicalEvent>,
    icloud: Option<&CanonicalEvent>,
    direction: SyncDirection,
    conflict_policy: ConflictPolicy,
) -> Vec<PlannedAction> {
    if let (Some(google), Some(icloud)) = (google, icloud) {
        let google_hash = hash_canonical_event(google);
        let icloud_hash = hash_canonical_event(icloud);
        if google_hash == icloud_hash {
            return vec![snapshot(
                uid,
                None,
                Some(google),
                Some(icloud),
                Some(google_hash),
                Some(icloud_hash),
            )];
        }

        return vec![resolve_provider_winner_conflict(ProviderWinnerInput {
            uid,
            reason: "unlinked_events_have_same_uid_but_differ",
            link: None,
            google,
            icloud,
            policy: conflict_policy,
            direction,
        })];
    }

    if let Some(google) = google
        && can_write_icloud(direction)
    {
        return vec![create_icloud(
            uid,
            google,
            None,
            Some(google),
            None,
            Some(hash_canonical_event(google)),
        )];
    }

    if let Some(icloud) = icloud
        && can_write_google(direction)
    {
        return vec![create_google(
            uid,
            icloud,
            None,
            None,
            Some(icloud),
            Some(hash_canonical_event(icloud)),
        )];
    }

    vec![snapshot(uid, None, google, icloud, None, None)]
}

struct ProviderWinnerInput<'a> {
    uid: &'a str,
    reason: &'a str,
    link: Option<&'a EventLink>,
    google: &'a CanonicalEvent,
    icloud: &'a CanonicalEvent,
    policy: ConflictPolicy,
    direction: SyncDirection,
}

fn resolve_provider_winner_conflict(input: ProviderWinnerInput<'_>) -> PlannedAction {
    match select_provider_winner(input.google, input.icloud, input.policy) {
        Some(ProviderWinner::Google) if can_write_icloud(input.direction) => update_icloud(
            input.uid,
            input.google,
            input.link,
            Some(input.google),
            Some(input.icloud),
            Some(hash_canonical_event(input.google)),
            Some(hash_canonical_event(input.icloud)),
            Some(AutoResolution {
                reason: input.reason.to_string(),
                policy: policy_name(input.policy).to_string(),
            }),
        ),
        Some(ProviderWinner::Icloud) if can_write_google(input.direction) => update_google(
            input.uid,
            input.icloud,
            input.link,
            Some(input.google),
            Some(input.icloud),
            Some(hash_canonical_event(input.google)),
            Some(hash_canonical_event(input.icloud)),
            Some(AutoResolution {
                reason: input.reason.to_string(),
                policy: policy_name(input.policy).to_string(),
            }),
        ),
        _ => manual_conflict(
            input.uid,
            input.reason,
            input.link,
            Some(input.google),
            Some(input.icloud),
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeletedSide {
    Google,
    Icloud,
}

struct DeleteUpdateInput<'a> {
    uid: &'a str,
    reason: &'a str,
    deleted_side: DeletedSide,
    changed_event: &'a CanonicalEvent,
    link: Option<&'a EventLink>,
    direction: SyncDirection,
    policy: DeleteConflictPolicy,
}

fn resolve_delete_update_conflict(input: DeleteUpdateInput<'_>) -> PlannedAction {
    let google = match input.deleted_side {
        DeletedSide::Icloud => Some(input.changed_event),
        DeletedSide::Google => None,
    };
    let icloud = match input.deleted_side {
        DeletedSide::Google => Some(input.changed_event),
        DeletedSide::Icloud => None,
    };

    match input.policy {
        DeleteConflictPolicy::Manual => {
            manual_conflict(input.uid, input.reason, input.link, google, icloud)
        }
        DeleteConflictPolicy::DeleteWins => match input.deleted_side {
            DeletedSide::Icloud if can_write_google(input.direction) => delete_google(
                input.uid,
                input.changed_event,
                input.link,
                google,
                icloud,
                Some(hash_canonical_event(input.changed_event)),
            )
            .with_resolution(input.reason, delete_policy_name(input.policy)),
            DeletedSide::Google if can_write_icloud(input.direction) => delete_icloud(
                input.uid,
                input.changed_event,
                input.link,
                google,
                icloud,
                Some(hash_canonical_event(input.changed_event)),
            )
            .with_resolution(input.reason, delete_policy_name(input.policy)),
            _ => manual_conflict(input.uid, input.reason, input.link, google, icloud),
        },
        DeleteConflictPolicy::UpdateWins => match input.deleted_side {
            DeletedSide::Icloud if can_write_icloud(input.direction) => create_icloud(
                input.uid,
                input.changed_event,
                input.link,
                google,
                icloud,
                Some(hash_canonical_event(input.changed_event)),
            )
            .with_resolution(input.reason, delete_policy_name(input.policy)),
            DeletedSide::Google if can_write_google(input.direction) => create_google(
                input.uid,
                input.changed_event,
                input.link,
                google,
                icloud,
                Some(hash_canonical_event(input.changed_event)),
            )
            .with_resolution(input.reason, delete_policy_name(input.policy)),
            _ => manual_conflict(input.uid, input.reason, input.link, google, icloud),
        },
    }
}

trait ActionResolutionExt {
    fn with_resolution(self, reason: &str, policy: &str) -> Self;
}

impl ActionResolutionExt for PlannedAction {
    fn with_resolution(mut self, reason: &str, policy: &str) -> Self {
        let resolution = Some(AutoResolution {
            reason: reason.to_string(),
            policy: policy.to_string(),
        });

        match &mut self {
            PlannedAction::CreateGoogle(action)
            | PlannedAction::CreateIcloud(action)
            | PlannedAction::UpdateGoogle(action)
            | PlannedAction::UpdateIcloud(action)
            | PlannedAction::DeleteGoogle(action)
            | PlannedAction::DeleteIcloud(action) => {
                action.resolution = resolution;
            }
            _ => {}
        }

        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderWinner {
    Google,
    Icloud,
}

fn select_provider_winner(
    google: &CanonicalEvent,
    icloud: &CanonicalEvent,
    policy: ConflictPolicy,
) -> Option<ProviderWinner> {
    match policy {
        ConflictPolicy::GoogleWins => Some(ProviderWinner::Google),
        ConflictPolicy::IcloudWins => Some(ProviderWinner::Icloud),
        ConflictPolicy::NewestUpdatedWins => match (
            google.provider_meta.updated_at,
            icloud.provider_meta.updated_at,
        ) {
            (Some(google_updated), Some(icloud_updated)) if google_updated > icloud_updated => {
                Some(ProviderWinner::Google)
            }
            (Some(google_updated), Some(icloud_updated)) if icloud_updated > google_updated => {
                Some(ProviderWinner::Icloud)
            }
            _ => None,
        },
        ConflictPolicy::Manual => None,
    }
}

fn current_events_by_uid(events: &[CanonicalEvent]) -> HashMap<String, &CanonicalEvent> {
    events
        .iter()
        .filter(|event| !event.provider_meta.deleted)
        .map(|event| (event.canonical_uid.clone(), event))
        .collect()
}

fn append_uids<'a, I>(ordered: &mut Vec<String>, seen: &mut HashSet<String>, uids: I)
where
    I: IntoIterator<Item = &'a String>,
{
    for uid in uids {
        if seen.insert(uid.clone()) {
            ordered.push(uid.clone());
        }
    }
}

fn snapshot(
    uid: &str,
    link: Option<&EventLink>,
    google: Option<&CanonicalEvent>,
    icloud: Option<&CanonicalEvent>,
    google_hash: Option<String>,
    icloud_hash: Option<String>,
) -> PlannedAction {
    PlannedAction::Snapshot {
        canonical_uid: uid.to_string(),
        link: link.cloned(),
        google: google.cloned().map(Box::new),
        icloud: icloud.cloned().map(Box::new),
        google_hash,
        icloud_hash,
        resolution: None,
    }
}

fn create_google(
    uid: &str,
    event: &CanonicalEvent,
    link: Option<&EventLink>,
    google: Option<&CanonicalEvent>,
    icloud: Option<&CanonicalEvent>,
    icloud_hash: Option<String>,
) -> PlannedAction {
    PlannedAction::CreateGoogle(mutating(
        uid,
        event,
        link,
        google,
        icloud,
        None,
        icloud_hash,
        None,
    ))
}

fn create_icloud(
    uid: &str,
    event: &CanonicalEvent,
    link: Option<&EventLink>,
    google: Option<&CanonicalEvent>,
    icloud: Option<&CanonicalEvent>,
    google_hash: Option<String>,
) -> PlannedAction {
    PlannedAction::CreateIcloud(mutating(
        uid,
        event,
        link,
        google,
        icloud,
        google_hash,
        None,
        None,
    ))
}

fn update_google(
    uid: &str,
    event: &CanonicalEvent,
    link: Option<&EventLink>,
    google: Option<&CanonicalEvent>,
    icloud: Option<&CanonicalEvent>,
    google_hash: Option<String>,
    icloud_hash: Option<String>,
    resolution: Option<AutoResolution>,
) -> PlannedAction {
    PlannedAction::UpdateGoogle(mutating(
        uid,
        event,
        link,
        google,
        icloud,
        google_hash,
        icloud_hash,
        resolution,
    ))
}

fn update_icloud(
    uid: &str,
    event: &CanonicalEvent,
    link: Option<&EventLink>,
    google: Option<&CanonicalEvent>,
    icloud: Option<&CanonicalEvent>,
    google_hash: Option<String>,
    icloud_hash: Option<String>,
    resolution: Option<AutoResolution>,
) -> PlannedAction {
    PlannedAction::UpdateIcloud(mutating(
        uid,
        event,
        link,
        google,
        icloud,
        google_hash,
        icloud_hash,
        resolution,
    ))
}

fn delete_google(
    uid: &str,
    event: &CanonicalEvent,
    link: Option<&EventLink>,
    google: Option<&CanonicalEvent>,
    icloud: Option<&CanonicalEvent>,
    google_hash: Option<String>,
) -> PlannedAction {
    PlannedAction::DeleteGoogle(mutating(
        uid,
        event,
        link,
        google,
        icloud,
        google_hash,
        None,
        None,
    ))
}

fn delete_icloud(
    uid: &str,
    event: &CanonicalEvent,
    link: Option<&EventLink>,
    google: Option<&CanonicalEvent>,
    icloud: Option<&CanonicalEvent>,
    icloud_hash: Option<String>,
) -> PlannedAction {
    PlannedAction::DeleteIcloud(mutating(
        uid,
        event,
        link,
        google,
        icloud,
        None,
        icloud_hash,
        None,
    ))
}

#[allow(clippy::too_many_arguments)]
fn mutating(
    uid: &str,
    event: &CanonicalEvent,
    link: Option<&EventLink>,
    google: Option<&CanonicalEvent>,
    icloud: Option<&CanonicalEvent>,
    google_hash: Option<String>,
    icloud_hash: Option<String>,
    resolution: Option<AutoResolution>,
) -> MutatingAction {
    MutatingAction {
        canonical_uid: uid.to_string(),
        event: Box::new(event.clone()),
        link: link.cloned(),
        google: google.cloned().map(Box::new),
        icloud: icloud.cloned().map(Box::new),
        google_hash,
        icloud_hash,
        resolution,
    }
}

fn manual_conflict(
    uid: &str,
    reason: &str,
    link: Option<&EventLink>,
    google: Option<&CanonicalEvent>,
    icloud: Option<&CanonicalEvent>,
) -> PlannedAction {
    PlannedAction::Conflict {
        canonical_uid: uid.to_string(),
        reason: reason.to_string(),
        resolution: ConflictResolution::Manual,
        link: link.cloned(),
        google: google.cloned().map(Box::new),
        icloud: icloud.cloned().map(Box::new),
    }
}

fn can_write_google(direction: SyncDirection) -> bool {
    matches!(
        direction,
        SyncDirection::TwoWay | SyncDirection::RightToLeft
    )
}

fn can_write_icloud(direction: SyncDirection) -> bool {
    matches!(
        direction,
        SyncDirection::TwoWay | SyncDirection::LeftToRight
    )
}

fn policy_name(policy: ConflictPolicy) -> &'static str {
    match policy {
        ConflictPolicy::Manual => "manual",
        ConflictPolicy::GoogleWins => "google_wins",
        ConflictPolicy::IcloudWins => "icloud_wins",
        ConflictPolicy::NewestUpdatedWins => "newest_updated_wins",
    }
}

fn delete_policy_name(policy: DeleteConflictPolicy) -> &'static str {
    match policy {
        DeleteConflictPolicy::Manual => "manual",
        DeleteConflictPolicy::DeleteWins => "delete_wins",
        DeleteConflictPolicy::UpdateWins => "update_wins",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EventAttendee, EventDateTime, EventReminder, EventStatus, EventVisibility,
        ProviderEventMeta, ProviderName, RecurrenceData,
    };
    use chrono::{TimeZone, Utc};
    use serde::Deserialize;

    fn event(uid: &str, title: &str) -> CanonicalEvent {
        CanonicalEvent {
            canonical_uid: uid.to_string(),
            title: title.to_string(),
            description: None,
            location: None,
            status: EventStatus::Confirmed,
            visibility: EventVisibility::Default,
            start: EventDateTime::DateTime {
                value: Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap(),
                timezone: Some("UTC".to_string()),
            },
            end: EventDateTime::DateTime {
                value: Utc.with_ymd_and_hms(2026, 6, 1, 13, 0, 0).unwrap(),
                timezone: Some("UTC".to_string()),
            },
            recurrence: None,
            attendees: Vec::new(),
            reminders: Vec::new(),
            provider_meta: ProviderEventMeta {
                provider: ProviderName::Google,
                calendar_id: "primary".to_string(),
                event_id: Some(uid.to_string()),
                href: None,
                etag: Some("etag".to_string()),
                ical_uid: Some(uid.to_string()),
                updated_at: None,
                deleted: false,
            },
            raw: serde_json::json!({}),
        }
    }

    fn link(uid: &str) -> EventLink {
        EventLink {
            id: format!("link-{uid}"),
            sync_pair_id: "personal".to_string(),
            canonical_uid: uid.to_string(),
            google_event_id: Some(uid.to_string()),
            google_ical_uid: Some(uid.to_string()),
            google_etag: Some("old-google-etag".to_string()),
            icloud_href: Some(format!(
                "https://caldav.icloud.com/calendars/example/{uid}.ics"
            )),
            icloud_uid: Some(uid.to_string()),
            icloud_etag: Some("old-icloud-etag".to_string()),
            google_hash: Some("old-google-hash".to_string()),
            icloud_hash: Some("old-icloud-hash".to_string()),
            last_synced_hash: Some("old-hash".to_string()),
            deleted_google_at: None,
            deleted_icloud_at: None,
        }
    }

    fn link_with_hashes(
        uid: &str,
        google_hash: Option<String>,
        icloud_hash: Option<String>,
    ) -> EventLink {
        EventLink {
            google_hash,
            icloud_hash,
            ..link(uid)
        }
    }

    fn action_resolution(action: &PlannedAction) -> Option<&AutoResolution> {
        match action {
            PlannedAction::CreateGoogle(action)
            | PlannedAction::CreateIcloud(action)
            | PlannedAction::UpdateGoogle(action)
            | PlannedAction::UpdateIcloud(action)
            | PlannedAction::DeleteGoogle(action)
            | PlannedAction::DeleteIcloud(action) => action.resolution.as_ref(),
            _ => None,
        }
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PlannerParityFixture {
        initial_cases: Vec<InitialFixtureCase>,
        two_way_cases: Vec<TwoWayFixtureCase>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct InitialFixtureCase {
        name: String,
        google_events: Vec<EventSpec>,
        icloud_events: Vec<EventSpec>,
        expected: Vec<ActionSummary>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TwoWayFixtureCase {
        name: String,
        direction: SyncDirection,
        conflict_policy: FixtureConflictPolicy,
        #[serde(default, rename = "knownICloudUidCollisions")]
        known_icloud_uid_collisions: Vec<String>,
        links: Vec<LinkSpec>,
        google_events: Vec<EventSpec>,
        icloud_events: Vec<EventSpec>,
        expected: Vec<ActionSummary>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureConflictPolicy {
        both_sides_changed: ConflictPolicy,
        unlinked_same_uid: ConflictPolicy,
        delete_vs_update: DeleteConflictPolicy,
        icloud_uid_collision: UidCollisionPolicy,
    }

    impl From<&FixtureConflictPolicy> for PlannerConflictPolicies {
        fn from(value: &FixtureConflictPolicy) -> Self {
            Self {
                both_sides_changed: value.both_sides_changed,
                unlinked_same_uid: value.unlinked_same_uid,
                delete_vs_update: value.delete_vs_update,
                icloud_uid_collision: value.icloud_uid_collision,
            }
        }
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct EventSpec {
        uid: String,
        title: String,
        updated_at: Option<String>,
        #[serde(default)]
        deleted: bool,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct LinkSpec {
        uid: String,
        google_hash: Option<String>,
        icloud_hash: Option<String>,
    }

    #[derive(Debug, PartialEq, Eq, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ActionSummary {
        kind: String,
        canonical_uid: String,
        #[serde(default)]
        reason: Option<String>,
        #[serde(default)]
        resolution: Option<String>,
        #[serde(default)]
        policy: Option<String>,
        #[serde(default)]
        event_title: Option<String>,
    }

    fn planner_parity_fixture() -> PlannerParityFixture {
        serde_json::from_str(include_str!("../../../test-fixtures/planner-parity.json"))
            .expect("planner parity fixture should parse")
    }

    fn fixture_event(provider: ProviderName, spec: &EventSpec) -> CanonicalEvent {
        CanonicalEvent {
            canonical_uid: spec.uid.clone(),
            title: spec.title.clone(),
            description: None,
            location: None,
            status: EventStatus::Confirmed,
            visibility: EventVisibility::Default,
            start: EventDateTime::DateTime {
                value: Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap(),
                timezone: Some("UTC".to_string()),
            },
            end: EventDateTime::DateTime {
                value: Utc.with_ymd_and_hms(2026, 6, 1, 13, 0, 0).unwrap(),
                timezone: Some("UTC".to_string()),
            },
            recurrence: None,
            attendees: Vec::new(),
            reminders: Vec::new(),
            provider_meta: ProviderEventMeta {
                provider,
                calendar_id: match provider {
                    ProviderName::Google => "primary",
                    ProviderName::Icloud => "icloud-calendar",
                }
                .to_string(),
                event_id: match provider {
                    ProviderName::Google => Some(spec.uid.clone()),
                    ProviderName::Icloud => None,
                },
                href: match provider {
                    ProviderName::Google => None,
                    ProviderName::Icloud => Some(format!(
                        "https://caldav.icloud.com/calendars/example/{}.ics",
                        spec.uid
                    )),
                },
                etag: Some(format!("{provider}-etag-{}", spec.uid)),
                ical_uid: Some(spec.uid.clone()),
                updated_at: spec.updated_at.as_ref().map(|value| {
                    chrono::DateTime::parse_from_rfc3339(value)
                        .unwrap()
                        .with_timezone(&Utc)
                }),
                deleted: spec.deleted,
            },
            raw: serde_json::json!({}),
        }
    }

    fn fixture_link(
        spec: &LinkSpec,
        google_events: &[CanonicalEvent],
        icloud_events: &[CanonicalEvent],
    ) -> EventLink {
        EventLink {
            id: format!("link-{}", spec.uid),
            sync_pair_id: "personal".to_string(),
            canonical_uid: spec.uid.clone(),
            google_event_id: Some(spec.uid.clone()),
            google_ical_uid: Some(spec.uid.clone()),
            google_etag: Some("old-google-etag".to_string()),
            icloud_href: Some(format!(
                "https://caldav.icloud.com/calendars/example/{}.ics",
                spec.uid
            )),
            icloud_uid: Some(spec.uid.clone()),
            icloud_etag: Some("old-icloud-etag".to_string()),
            google_hash: resolve_fixture_hash(
                spec.google_hash.as_deref(),
                "google",
                google_events,
                icloud_events,
            ),
            icloud_hash: resolve_fixture_hash(
                spec.icloud_hash.as_deref(),
                "icloud",
                google_events,
                icloud_events,
            ),
            last_synced_hash: Some("old-hash".to_string()),
            deleted_google_at: None,
            deleted_icloud_at: None,
        }
    }

    fn resolve_fixture_hash(
        value: Option<&str>,
        side: &str,
        google_events: &[CanonicalEvent],
        icloud_events: &[CanonicalEvent],
    ) -> Option<String> {
        let value = value?;
        let prefix = format!("$hash:{side}:");
        if !value.starts_with(&prefix) {
            return Some(value.to_string());
        }

        let uid = value.trim_start_matches(&prefix);
        let events = if side == "google" {
            google_events
        } else {
            icloud_events
        };
        let event = events
            .iter()
            .find(|event| event.canonical_uid == uid)
            .unwrap_or_else(|| panic!("missing {side} fixture event for hash placeholder {value}"));
        Some(hash_canonical_event(event))
    }

    fn action_summary(action: &PlannedAction) -> ActionSummary {
        let (kind, canonical_uid, event, auto_resolution) = match action {
            PlannedAction::Snapshot { canonical_uid, .. } => {
                ("snapshot", canonical_uid, None, None)
            }
            PlannedAction::Noop { canonical_uid, .. } => ("noop", canonical_uid, None, None),
            PlannedAction::CreateGoogle(action) => (
                "create_google",
                &action.canonical_uid,
                Some(action.event.as_ref()),
                action.resolution.as_ref(),
            ),
            PlannedAction::CreateIcloud(action) => (
                "create_icloud",
                &action.canonical_uid,
                Some(action.event.as_ref()),
                action.resolution.as_ref(),
            ),
            PlannedAction::UpdateGoogle(action) => (
                "update_google",
                &action.canonical_uid,
                Some(action.event.as_ref()),
                action.resolution.as_ref(),
            ),
            PlannedAction::UpdateIcloud(action) => (
                "update_icloud",
                &action.canonical_uid,
                Some(action.event.as_ref()),
                action.resolution.as_ref(),
            ),
            PlannedAction::DeleteGoogle(action) => (
                "delete_google",
                &action.canonical_uid,
                Some(action.event.as_ref()),
                action.resolution.as_ref(),
            ),
            PlannedAction::DeleteIcloud(action) => (
                "delete_icloud",
                &action.canonical_uid,
                Some(action.event.as_ref()),
                action.resolution.as_ref(),
            ),
            PlannedAction::Conflict {
                canonical_uid,
                reason,
                resolution,
                ..
            } => {
                return ActionSummary {
                    kind: "conflict".to_string(),
                    canonical_uid: canonical_uid.clone(),
                    reason: Some(reason.clone()),
                    resolution: Some(
                        match resolution {
                            ConflictResolution::Manual => "manual",
                            ConflictResolution::Ignored => "ignored",
                        }
                        .to_string(),
                    ),
                    policy: None,
                    event_title: None,
                };
            }
        };

        ActionSummary {
            kind: kind.to_string(),
            canonical_uid: canonical_uid.clone(),
            reason: match action {
                PlannedAction::Noop { reason, .. } => Some(reason.clone()),
                _ => auto_resolution.map(|resolution| resolution.reason.clone()),
            },
            resolution: auto_resolution.map(|_| "auto_resolved".to_string()),
            policy: auto_resolution.map(|resolution| resolution.policy.clone()),
            event_title: event.map(|event| event.title.clone()),
        }
    }

    #[test]
    fn initial_sync_creates_missing_sides() {
        let google = event("shared-uid", "Lunch");
        let icloud = event("icloud-only", "Dentist");

        let actions = plan_initial_actions(&[google], &[icloud]);

        assert!(matches!(actions[0], PlannedAction::CreateIcloud(_)));
        assert!(matches!(actions[1], PlannedAction::CreateGoogle(_)));
    }

    #[test]
    fn initial_sync_noops_when_present_on_both_sides() {
        let google = event("shared-uid", "Lunch");
        let icloud = event("shared-uid", "Lunch");

        let actions = plan_initial_actions(&[google], &[icloud]);

        assert!(matches!(actions[0], PlannedAction::Noop { .. }));
    }

    #[test]
    fn planner_matches_shared_initial_parity_fixtures() {
        for item in planner_parity_fixture().initial_cases {
            let google_events = item
                .google_events
                .iter()
                .map(|event| fixture_event(ProviderName::Google, event))
                .collect::<Vec<_>>();
            let icloud_events = item
                .icloud_events
                .iter()
                .map(|event| fixture_event(ProviderName::Icloud, event))
                .collect::<Vec<_>>();
            let actions = plan_initial_actions(&google_events, &icloud_events)
                .iter()
                .map(action_summary)
                .collect::<Vec<_>>();

            assert_eq!(actions, item.expected, "initial fixture: {}", item.name);
        }
    }

    #[test]
    fn planner_matches_shared_two_way_parity_fixtures() {
        for item in planner_parity_fixture().two_way_cases {
            let google_events = item
                .google_events
                .iter()
                .map(|event| fixture_event(ProviderName::Google, event))
                .collect::<Vec<_>>();
            let icloud_events = item
                .icloud_events
                .iter()
                .map(|event| fixture_event(ProviderName::Icloud, event))
                .collect::<Vec<_>>();
            let links = item
                .links
                .iter()
                .map(|link| fixture_link(link, &google_events, &icloud_events))
                .collect::<Vec<_>>();
            let actions = plan_two_way_actions(PlanTwoWayInput {
                links: &links,
                google_events: &google_events,
                icloud_events: &icloud_events,
                known_icloud_uid_collisions: item
                    .known_icloud_uid_collisions
                    .iter()
                    .cloned()
                    .collect(),
                direction: item.direction,
                conflict_policy: PlannerConflictPolicies::from(&item.conflict_policy),
            })
            .iter()
            .map(action_summary)
            .collect::<Vec<_>>();

            assert_eq!(actions, item.expected, "two-way fixture: {}", item.name);
        }
    }

    #[test]
    fn hash_ignores_provider_metadata() {
        let base = event("shared-uid", "Lunch");
        let mut moved_provider_meta = base.clone();
        moved_provider_meta.provider_meta.etag = Some("changed".to_string());

        assert_eq!(
            hash_canonical_event(&base),
            hash_canonical_event(&moved_provider_meta)
        );
    }

    #[test]
    fn hash_treats_equivalent_instants_as_equal() {
        let mut google = event("shared-uid", "Lunch");
        google.start = EventDateTime::DateTime {
            value: chrono::DateTime::parse_from_rfc3339("2026-06-06T12:00:00+02:00")
                .unwrap()
                .with_timezone(&Utc),
            timezone: Some("Europe/Berlin".to_string()),
        };
        google.end = EventDateTime::DateTime {
            value: chrono::DateTime::parse_from_rfc3339("2026-06-06T13:00:00+02:00")
                .unwrap()
                .with_timezone(&Utc),
            timezone: Some("Europe/Berlin".to_string()),
        };
        google.recurrence = Some(RecurrenceData {
            sequence: Some(1),
            ..RecurrenceData::default()
        });

        let mut icloud = event("shared-uid", "Lunch");
        icloud.start = EventDateTime::DateTime {
            value: chrono::DateTime::parse_from_rfc3339("2026-06-06T10:00:00.000Z")
                .unwrap()
                .with_timezone(&Utc),
            timezone: Some("Europe/Berlin".to_string()),
        };
        icloud.end = EventDateTime::DateTime {
            value: chrono::DateTime::parse_from_rfc3339("2026-06-06T11:00:00.000Z")
                .unwrap()
                .with_timezone(&Utc),
            timezone: Some("Europe/Berlin".to_string()),
        };
        icloud.recurrence = Some(RecurrenceData {
            sequence: Some(5),
            ..RecurrenceData::default()
        });

        assert_eq!(hash_canonical_event(&google), hash_canonical_event(&icloud));
    }

    #[test]
    fn hash_ignores_harmless_text_differences() {
        let mut google = event("shared-uid", "Lunch");
        google.description = Some("Bring notes\r\n".to_string());
        google.location = Some("Room 1\\nFloor 2".to_string());

        let mut icloud = event("shared-uid", "Lunch");
        icloud.description = Some("Bring notes".to_string());
        icloud.location = Some("Room 1\nFloor 2".to_string());

        assert_eq!(hash_canonical_event(&google), hash_canonical_event(&icloud));
    }

    #[test]
    fn hash_ignores_provider_only_fields() {
        let mut google = event("shared-uid", "Lunch");
        google.visibility = EventVisibility::Public;
        google.reminders = vec![EventReminder {
            method: "email".to_string(),
            minutes_before_start: 10,
        }];

        let mut icloud = event("shared-uid", "Lunch");
        icloud.visibility = EventVisibility::Default;
        icloud.attendees = vec![EventAttendee {
            email: "/apple-principal/".to_string(),
            name: Some("Apple Principal".to_string()),
            response_status: Some("accepted".to_string()),
            optional: false,
        }];

        assert_eq!(hash_canonical_event(&google), hash_canonical_event(&icloud));
    }

    #[test]
    fn hash_ignores_timezone_labels_when_instants_match() {
        let mut google = event("shared-uid", "Flight");
        google.start = EventDateTime::DateTime {
            value: chrono::DateTime::parse_from_rfc3339("2025-03-19T16:55:00.000Z")
                .unwrap()
                .with_timezone(&Utc),
            timezone: Some("UTC".to_string()),
        };
        google.end = EventDateTime::DateTime {
            value: chrono::DateTime::parse_from_rfc3339("2025-03-19T19:20:00.000Z")
                .unwrap()
                .with_timezone(&Utc),
            timezone: Some("UTC".to_string()),
        };

        let mut icloud = event("shared-uid", "Flight");
        icloud.start = EventDateTime::DateTime {
            value: chrono::DateTime::parse_from_rfc3339("2025-03-19T17:55:00+01:00")
                .unwrap()
                .with_timezone(&Utc),
            timezone: Some("GMT+0100".to_string()),
        };
        icloud.end = EventDateTime::DateTime {
            value: chrono::DateTime::parse_from_rfc3339("2025-03-19T20:20:00+01:00")
                .unwrap()
                .with_timezone(&Utc),
            timezone: Some("GMT+0100".to_string()),
        };

        assert_eq!(hash_canonical_event(&google), hash_canonical_event(&icloud));
    }

    #[test]
    fn auto_resolves_both_side_changes_with_newest_updated_wins() {
        let mut google = event("shared-uid", "New Google Title");
        google.provider_meta.updated_at = Some(Utc.with_ymd_and_hms(2026, 6, 2, 10, 0, 0).unwrap());

        let mut icloud = event("shared-uid", "Old iCloud Title");
        icloud.provider_meta.updated_at = Some(Utc.with_ymd_and_hms(2026, 6, 2, 9, 0, 0).unwrap());

        let actions = plan_two_way_actions(PlanTwoWayInput {
            links: &[link("shared-uid")],
            google_events: &[google],
            icloud_events: &[icloud],
            known_icloud_uid_collisions: HashSet::new(),
            direction: SyncDirection::TwoWay,
            conflict_policy: PlannerConflictPolicies {
                both_sides_changed: ConflictPolicy::NewestUpdatedWins,
                ..PlannerConflictPolicies::default()
            },
        });

        match &actions[0] {
            PlannedAction::UpdateIcloud(action) => {
                let resolution = action.resolution.as_ref().unwrap();
                assert_eq!(resolution.reason, "both_sides_changed");
                assert_eq!(resolution.policy, "newest_updated_wins");
            }
            action => panic!("unexpected action: {action:?}"),
        }
    }

    #[test]
    fn auto_resolves_delete_update_with_update_wins() {
        let google = event("shared-uid", "Changed on Google");
        let actions = plan_two_way_actions(PlanTwoWayInput {
            links: &[link("shared-uid")],
            google_events: &[google],
            icloud_events: &[],
            known_icloud_uid_collisions: HashSet::new(),
            direction: SyncDirection::TwoWay,
            conflict_policy: PlannerConflictPolicies {
                delete_vs_update: DeleteConflictPolicy::UpdateWins,
                ..PlannerConflictPolicies::default()
            },
        });

        match &actions[0] {
            PlannedAction::CreateIcloud(action) => {
                let resolution = action.resolution.as_ref().unwrap();
                assert_eq!(resolution.reason, "google_changed_while_icloud_deleted");
                assert_eq!(resolution.policy, "update_wins");
            }
            action => panic!("unexpected action: {action:?}"),
        }
    }

    #[test]
    fn linked_google_change_updates_icloud() {
        let google = event("shared-uid", "Changed on Google");
        let icloud = event("shared-uid", "Old Title");
        let link = link_with_hashes(
            "shared-uid",
            Some("old-google-hash".to_string()),
            Some(hash_canonical_event(&icloud)),
        );

        let actions = plan_two_way_actions(PlanTwoWayInput {
            links: &[link],
            google_events: &[google],
            icloud_events: &[icloud],
            known_icloud_uid_collisions: HashSet::new(),
            direction: SyncDirection::TwoWay,
            conflict_policy: PlannerConflictPolicies::default(),
        });

        assert!(matches!(actions[0], PlannedAction::UpdateIcloud(_)));
    }

    #[test]
    fn linked_icloud_change_updates_google() {
        let google = event("shared-uid", "Old Title");
        let icloud = event("shared-uid", "Changed on iCloud");
        let link = link_with_hashes(
            "shared-uid",
            Some(hash_canonical_event(&google)),
            Some("old-icloud-hash".to_string()),
        );

        let actions = plan_two_way_actions(PlanTwoWayInput {
            links: &[link],
            google_events: &[google],
            icloud_events: &[icloud],
            known_icloud_uid_collisions: HashSet::new(),
            direction: SyncDirection::TwoWay,
            conflict_policy: PlannerConflictPolicies::default(),
        });

        assert!(matches!(actions[0], PlannedAction::UpdateGoogle(_)));
    }

    #[test]
    fn linked_equal_events_snapshot() {
        let google = event("shared-uid", "Lunch");
        let icloud = event("shared-uid", "Lunch");
        let hash = hash_canonical_event(&google);
        let link = link_with_hashes("shared-uid", Some(hash.clone()), Some(hash));

        let actions = plan_two_way_actions(PlanTwoWayInput {
            links: &[link],
            google_events: &[google],
            icloud_events: &[icloud],
            known_icloud_uid_collisions: HashSet::new(),
            direction: SyncDirection::TwoWay,
            conflict_policy: PlannerConflictPolicies::default(),
        });

        assert!(matches!(actions[0], PlannedAction::Snapshot { .. }));
    }

    #[test]
    fn both_side_changes_are_manual_by_default() {
        let google = event("shared-uid", "Changed on Google");
        let icloud = event("shared-uid", "Changed on iCloud");

        let actions = plan_two_way_actions(PlanTwoWayInput {
            links: &[link("shared-uid")],
            google_events: &[google],
            icloud_events: &[icloud],
            known_icloud_uid_collisions: HashSet::new(),
            direction: SyncDirection::TwoWay,
            conflict_policy: PlannerConflictPolicies::default(),
        });

        match &actions[0] {
            PlannedAction::Conflict {
                reason, resolution, ..
            } => {
                assert_eq!(reason, "both_sides_changed");
                assert_eq!(*resolution, ConflictResolution::Manual);
            }
            action => panic!("unexpected action: {action:?}"),
        }
    }

    #[test]
    fn provider_winner_policies_resolve_both_side_changes() {
        let google = event("shared-uid", "Changed on Google");
        let icloud = event("shared-uid", "Changed on iCloud");

        let google_wins = plan_two_way_actions(PlanTwoWayInput {
            links: &[link("shared-uid")],
            google_events: std::slice::from_ref(&google),
            icloud_events: std::slice::from_ref(&icloud),
            known_icloud_uid_collisions: HashSet::new(),
            direction: SyncDirection::TwoWay,
            conflict_policy: PlannerConflictPolicies {
                both_sides_changed: ConflictPolicy::GoogleWins,
                ..PlannerConflictPolicies::default()
            },
        });

        assert!(matches!(google_wins[0], PlannedAction::UpdateIcloud(_)));
        assert_eq!(
            action_resolution(&google_wins[0]).unwrap().policy,
            "google_wins"
        );

        let icloud_wins = plan_two_way_actions(PlanTwoWayInput {
            links: &[link("shared-uid")],
            google_events: &[google],
            icloud_events: &[icloud],
            known_icloud_uid_collisions: HashSet::new(),
            direction: SyncDirection::TwoWay,
            conflict_policy: PlannerConflictPolicies {
                both_sides_changed: ConflictPolicy::IcloudWins,
                ..PlannerConflictPolicies::default()
            },
        });

        assert!(matches!(icloud_wins[0], PlannedAction::UpdateGoogle(_)));
        assert_eq!(
            action_resolution(&icloud_wins[0]).unwrap().policy,
            "icloud_wins"
        );
    }

    #[test]
    fn unlinked_same_uid_differing_events_can_auto_resolve() {
        let google = event("shared-uid", "Google Title");
        let icloud = event("shared-uid", "iCloud Title");

        let actions = plan_two_way_actions(PlanTwoWayInput {
            links: &[],
            google_events: &[google],
            icloud_events: &[icloud],
            known_icloud_uid_collisions: HashSet::new(),
            direction: SyncDirection::TwoWay,
            conflict_policy: PlannerConflictPolicies {
                unlinked_same_uid: ConflictPolicy::GoogleWins,
                ..PlannerConflictPolicies::default()
            },
        });

        match &actions[0] {
            PlannedAction::UpdateIcloud(action) => {
                let resolution = action.resolution.as_ref().unwrap();
                assert_eq!(
                    resolution.reason,
                    "unlinked_events_have_same_uid_but_differ"
                );
                assert_eq!(resolution.policy, "google_wins");
            }
            action => panic!("unexpected action: {action:?}"),
        }
    }

    #[test]
    fn delete_wins_removes_changed_remaining_side() {
        let google = event("shared-uid", "Changed on Google");

        let actions = plan_two_way_actions(PlanTwoWayInput {
            links: &[link("shared-uid")],
            google_events: &[google],
            icloud_events: &[],
            known_icloud_uid_collisions: HashSet::new(),
            direction: SyncDirection::TwoWay,
            conflict_policy: PlannerConflictPolicies {
                delete_vs_update: DeleteConflictPolicy::DeleteWins,
                ..PlannerConflictPolicies::default()
            },
        });

        match &actions[0] {
            PlannedAction::DeleteGoogle(action) => {
                let resolution = action.resolution.as_ref().unwrap();
                assert_eq!(resolution.reason, "google_changed_while_icloud_deleted");
                assert_eq!(resolution.policy, "delete_wins");
            }
            action => panic!("unexpected action: {action:?}"),
        }
    }

    #[test]
    fn direction_prevents_backfill_against_configured_flow() {
        let icloud = event("icloud-only", "Dentist");

        let actions = plan_two_way_actions(PlanTwoWayInput {
            links: &[],
            google_events: &[],
            icloud_events: &[icloud],
            known_icloud_uid_collisions: HashSet::new(),
            direction: SyncDirection::LeftToRight,
            conflict_policy: PlannerConflictPolicies::default(),
        });

        assert!(matches!(actions[0], PlannedAction::Snapshot { .. }));
    }

    #[test]
    fn known_icloud_uid_collision_can_be_ignored_or_manual() {
        let google = event("shared-uid", "Google Title");
        let known = HashSet::from(["shared-uid".to_string()]);

        let ignored = plan_two_way_actions(PlanTwoWayInput {
            links: &[],
            google_events: std::slice::from_ref(&google),
            icloud_events: &[],
            known_icloud_uid_collisions: known.clone(),
            direction: SyncDirection::TwoWay,
            conflict_policy: PlannerConflictPolicies {
                icloud_uid_collision: UidCollisionPolicy::IgnoreKnown,
                ..PlannerConflictPolicies::default()
            },
        });

        match &ignored[0] {
            PlannedAction::Conflict {
                reason, resolution, ..
            } => {
                assert_eq!(reason, "icloud_uid_exists_in_different_calendar");
                assert_eq!(*resolution, ConflictResolution::Ignored);
            }
            action => panic!("unexpected action: {action:?}"),
        }

        let manual = plan_two_way_actions(PlanTwoWayInput {
            links: &[],
            google_events: &[google],
            icloud_events: &[],
            known_icloud_uid_collisions: known,
            direction: SyncDirection::TwoWay,
            conflict_policy: PlannerConflictPolicies {
                icloud_uid_collision: UidCollisionPolicy::Manual,
                ..PlannerConflictPolicies::default()
            },
        });

        match &manual[0] {
            PlannedAction::Conflict { resolution, .. } => {
                assert_eq!(*resolution, ConflictResolution::Manual);
            }
            action => panic!("unexpected action: {action:?}"),
        }
    }
}
