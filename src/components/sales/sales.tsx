import { component$ } from "@builder.io/qwik";
import type {
  BdStatus,
  BookingStatus,
  DeliveryProgress,
  DrInfo,
  DrStatus,
  PriceSource,
  VatAmounts,
} from "~/lib/bindings";
import { formatPct, formatPeso } from "~/lib/format";
import { Badge } from "~/components/ui/ui";

type Tone = "brand" | "muted" | "danger" | "warning";

export const BD_STATUS: Record<BdStatus, { label: string; tone: Tone }> = {
  open: { label: "Open", tone: "warning" },
  quoted: { label: "Quoted", tone: "brand" },
  won: { label: "Won", tone: "brand" },
  lost: { label: "Lost", tone: "muted" },
};

export const BOOKING_STATUS: Record<BookingStatus, { label: string; tone: Tone }> = {
  draft: { label: "Draft", tone: "warning" },
  confirmed: { label: "Confirmed", tone: "brand" },
  closed: { label: "Closed", tone: "muted" },
  cancelled: { label: "Cancelled", tone: "muted" },
};

export const DR_STATUS: Record<DrStatus, { label: string; tone: Tone }> = {
  requested: { label: "Requested", tone: "warning" },
  approved: { label: "Approved", tone: "brand" },
  delivered: { label: "Delivered", tone: "brand" },
  cancelled: { label: "Cancelled", tone: "muted" },
};

export const PROGRESS_LABEL: Record<DeliveryProgress, string> = {
  none: "Not delivered",
  partial: "Partly delivered",
  full: "Delivered",
};

export const PRICE_SOURCE_LABEL: Record<PriceSource, string> = {
  manual: "Client rate",
  client_tier: "Client's margin tier",
  item_default: "Item's default margin",
  quoted: "Quoted in BD",
  custom: "Typed in",
};

export const StatusBadge = component$<{ s: { label: string; tone: Tone } }>(({ s }) => (
  <Badge tone={s.tone}>{s.label}</Badge>
));

export const DrStatusBadge = component$<{ d: DrInfo }>(({ d }) =>
  d.voided ? <Badge tone="danger">Void</Badge> : <StatusBadge s={DR_STATUS[d.status]} />,
);

/** "Offline no." badge for numbers given while other PCs were unreachable. */
export const OfflineBadge = component$<{ show: boolean }>(({ show }) =>
  show ? (
    <span class="ml-2" title="Given while offline from other PCs; it may still change.">
      <Badge tone="warning">offline no.</Badge>
    </span>
  ) : null,
);

/** A DR's number with its warnings, or a placeholder. */
export const DrNo = component$<{ d: DrInfo }>(({ d }) => (
  <>
    {d.no ?? <span class="text-ink-muted">No DR no. yet</span>}
    {d.no_clash && (
      <span class="ml-2" title="Another DR has the same number. Correct the wrong one.">
        <Badge tone="danger">used twice</Badge>
      </span>
    )}
    {d.no && !d.no_confirmed && !d.no_clash && (
      <span class="ml-2" title="Entered while offline from other PCs: not checked against them yet.">
        <Badge tone="warning">unchecked</Badge>
      </span>
    )}
  </>
));

/** Net, VAT and gross of a document. */
export const Totals = component$<{ totals: VatAmounts; vat: boolean; vatBp: number }>(
  ({ totals, vat, vatBp }) => (
    <dl class="ml-auto grid w-72 grid-cols-[1fr_auto] gap-x-4 gap-y-1 text-sm">
      <dt class="text-ink-muted">Net</dt>
      <dd class="text-right tabular-nums">{formatPeso(totals.net_c)}</dd>
      <dt class="text-ink-muted">{vat ? `VAT (${formatPct(vatBp)})` : "VAT"}</dt>
      <dd class="text-right tabular-nums">{vat ? formatPeso(totals.vat_c) : "none"}</dd>
      <dt class="font-semibold">Gross</dt>
      <dd class="text-right font-semibold tabular-nums">{formatPeso(totals.gross_c)}</dd>
    </dl>
  ),
);
