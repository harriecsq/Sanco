import { component$ } from "@builder.io/qwik";
import { Badge } from "~/components/ui/ui";

/** Days of stock left before expiry, by default within this many days. */
export const NEAR_EXPIRY_DAYS = 90;

/** "expired", "12 days" (near expiry) or nothing. */
export const ExpiryBadge = component$<{ days: number | null | undefined; within?: number }>(
  ({ days, within = NEAR_EXPIRY_DAYS }) => {
    if (days == null || days > within) return null;
    if (days < 0) return <Badge tone="danger">expired</Badge>;
    if (days === 0) return <Badge tone="danger">expires today</Badge>;
    return <Badge tone="warning">{days === 1 ? "1 day" : `${days} days`}</Badge>;
  },
);
