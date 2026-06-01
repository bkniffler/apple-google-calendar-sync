import type { CanonicalEvent } from "../providers/types";

export type SyncAction =
  | {
      kind: "noop";
      canonicalUid: string;
      reason: string;
    }
  | {
      kind: "create_google" | "create_icloud" | "update_google" | "update_icloud";
      canonicalUid: string;
      event: CanonicalEvent;
    }
  | {
      kind: "delete_google" | "delete_icloud";
      canonicalUid: string;
      remoteEventId: string;
      etag?: string;
    }
  | {
      kind: "conflict";
      canonicalUid: string;
      reason: string;
      google?: CanonicalEvent;
      icloud?: CanonicalEvent;
    };

export function planInitialActions(
  googleEvents: CanonicalEvent[],
  icloudEvents: CanonicalEvent[]
): SyncAction[] {
  const actions: SyncAction[] = [];
  const googleByUid = new Map(googleEvents.map((event) => [event.canonicalUid, event]));
  const icloudByUid = new Map(icloudEvents.map((event) => [event.canonicalUid, event]));
  const uids = new Set([...googleByUid.keys(), ...icloudByUid.keys()]);

  for (const uid of uids) {
    const google = googleByUid.get(uid);
    const icloud = icloudByUid.get(uid);

    if (google && !icloud) {
      actions.push({ kind: "create_icloud", canonicalUid: uid, event: google });
      continue;
    }

    if (!google && icloud) {
      actions.push({ kind: "create_google", canonicalUid: uid, event: icloud });
      continue;
    }

    actions.push({ kind: "noop", canonicalUid: uid, reason: "present_on_both_sides" });
  }

  return actions;
}
