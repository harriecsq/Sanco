// Display helpers. Timestamps are UTC ms shown in Manila time; money is
// integer centavos.

const TZ = "Asia/Manila";

const dateTime = new Intl.DateTimeFormat("en-PH", {
  timeZone: TZ,
  dateStyle: "medium",
  timeStyle: "short",
});
const dateOnly = new Intl.DateTimeFormat("en-PH", {
  timeZone: TZ,
  dateStyle: "medium",
});
const peso = new Intl.NumberFormat("en-PH", {
  style: "currency",
  currency: "PHP",
  minimumFractionDigits: 2,
});

export const formatDateTime = (ms: number | null | undefined) =>
  ms ? dateTime.format(ms) : "—";

export const formatDate = (ms: number | null | undefined) =>
  ms ? dateOnly.format(ms) : "—";

/** "3 min ago", "2 h ago", or the date for anything older than a day. */
export const formatAgo = (ms: number | null | undefined, now = Date.now()) => {
  if (!ms) return "never";
  const s = Math.round((now - ms) / 1000);
  if (s < 45) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  return formatDateTime(ms);
};

/** "just now", "5 min ago", "3 h ago", "6 days ago", "2 months ago". */
export const formatSince = (ms: number, now = Date.now()) => {
  const min = Math.floor((now - ms) / 60000);
  if (min < 1) return "just now";
  if (min < 60) return `${min} min ago`;
  const h = Math.floor(min / 60);
  if (h < 24) return `${h} h ago`;
  const d = Math.floor(h / 24);
  if (d < 60) return d === 1 ? "1 day ago" : `${d} days ago`;
  const mo = Math.floor(d / 30);
  return mo < 24 ? `${mo} months ago` : `${Math.floor(d / 365)} years ago`;
};

/** ₱1,234.50 */
export const formatPeso = (centavos: number) => peso.format(centavos / 100);

export const formatBytes = (n: number) => {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
};

/** Basis points as a percentage: 2000 → "20%", 1250 → "12.5%". */
export const formatPct = (bp: number | null | undefined) =>
  bp == null ? "—" : `${(bp / 100).toLocaleString("en-PH", { maximumFractionDigits: 2 })}%`;

/** Centavos, or "—" when missing. */
export const formatPesoOr = (centavos: number | null | undefined) =>
  centavos == null ? "—" : formatPeso(centavos);

/** Minor units of a currency: 1000 USD → "US$10.00". */
export const formatMinor = (minor: number | null | undefined, currency: string) =>
  minor == null
    ? "—"
    : new Intl.NumberFormat("en-PH", {
        style: "currency",
        currency,
        minimumFractionDigits: 2,
      }).format(minor / 100);

const dayFmt = new Intl.DateTimeFormat("en-PH", { timeZone: "UTC", dateStyle: "medium" });

/** A calendar date `YYYY-MM-DD` as "Oct 21, 2026". */
export const formatDay = (date: string | null | undefined) => {
  if (!date) return "—";
  const [y, m, d] = date.split("-").map(Number);
  return Number.isFinite(d) ? dayFmt.format(Date.UTC(y, m - 1, d)) : date;
};

/** Today's date in Manila, `YYYY-MM-DD`. */
export const todayManila = () =>
  new Intl.DateTimeFormat("en-CA", { timeZone: TZ }).format(Date.now());

/** `YYYY-MM-DD` plus `days` (may be negative). */
export const addDays = (date: string, days: number) => {
  const [y, m, d] = date.split("-").map(Number);
  return new Date(Date.UTC(y, m - 1, d + days)).toISOString().slice(0, 10);
};

/** A quantity with its unit: "1,200 box". */
export const formatQty = (qty: number, unit?: string | null) =>
  `${qty.toLocaleString("en-PH")}${unit ? ` ${unit}` : ""}`;

/** Box equivalent of a quantity: "9 boxes + 7" for 97 at 10 a box. Empty
 * without a box size. */
export const boxEquiv = (qty: number, perBox: number | null | undefined) => {
  if (!perBox || perBox <= 0 || qty <= 0) return "";
  const full = Math.floor(qty / perBox);
  const rest = qty % perBox;
  const boxes = full ? `${full.toLocaleString("en-PH")} ${full === 1 ? "box" : "boxes"}` : "";
  return [boxes, rest ? (full ? `+ ${rest}` : `${rest} loose`) : ""].filter(Boolean).join(" ");
};

/** Boxes in stock: "9 full + 1 open (7)". Empty when there are none. */
export const boxSummary = (full: number, open: number, openPcs: number) => {
  const parts = [];
  if (full) parts.push(`${full.toLocaleString("en-PH")} full`);
  if (open) parts.push(`${open.toLocaleString("en-PH")} open (${openPcs.toLocaleString("en-PH")})`);
  return parts.join(" + ");
};

/** "MT26-0001 – MT26-0010" for a list of serials in order. */
export const serialRange = (serials: (string | null)[]) => {
  const s = serials.filter((x): x is string => !!x);
  if (!s.length) return "";
  return s.length === 1 ? s[0] : `${s[0]} – ${s[s.length - 1]}`;
};

/** The box to break first: the earliest expiry, then the first listed
 *  (boxes come lowest serial first). Undefined for an empty list. */
export const oldestBox = (boxes: { box_id: string; expiry: string | null }[]) =>
  boxes.reduce<(typeof boxes)[number] | undefined>(
    (best, b) => (!best || (b.expiry ?? "9999") < (best.expiry ?? "9999") ? b : best),
    undefined,
  )?.box_id;

/** Thrown by the parsers below; the message is shown to the user. */
export class InputError extends Error {}

/** "1,234.50" → 123450 (hundredths). Blank → null. */
export function parseMoney(text: string, what = "amount"): number | null {
  const t = text.trim().replace(/[₱,\s]/g, "");
  if (!t) return null;
  if (!/^-?\d+(\.\d{0,2})?$/.test(t)) {
    throw new InputError(`“${text.trim()}” is not a valid ${what}`);
  }
  const [whole, frac = ""] = t.replace("-", "").split(".");
  const v = Number(whole) * 100 + Number(frac.padEnd(2, "0"));
  return t.startsWith("-") ? -v : v;
}

/** "12.5" or "12.5%" → 1250 basis points. Blank → null. */
export function parsePct(text: string, what = "percentage"): number | null {
  const t = text.trim().replace(/[%\s]/g, "");
  if (!t) return null;
  if (!/^\d+(\.\d{0,2})?$/.test(t)) {
    throw new InputError(`“${text.trim()}” is not a valid ${what}`);
  }
  const [whole, frac = ""] = t.split(".");
  return Number(whole) * 100 + Number(frac.padEnd(2, "0"));
}

/** Whole number, blank → null. */
export function parseCount(text: string, what = "number"): number | null {
  const t = text.trim().replace(/,/g, "");
  if (!t) return null;
  if (!/^\d+$/.test(t)) throw new InputError(`“${text.trim()}” is not a valid ${what}`);
  return Number(t);
}

/** Whole number that may be signed ("+5", "-3", "−3"), blank → null. */
export function parseSigned(text: string, what = "quantity"): number | null {
  const t = text.trim().replace(/,/g, "").replace("−", "-");
  if (!t) return null;
  if (!/^[+-]?\d+$/.test(t)) throw new InputError(`“${text.trim()}” is not a valid ${what}`);
  return Number(t);
}

/** Hundredths → input text: 123450 → "1234.50". */
export const moneyText = (v: number | null | undefined) =>
  v == null ? "" : (v / 100).toFixed(2);

/** Basis points → input text: 1250 → "12.5". */
export const pctText = (bp: number | null | undefined) =>
  bp == null ? "" : String(bp / 100);

/** Price at a margin on price, like the backend: cost ÷ (1 − margin). */
export const priceAtMargin = (cost: number, bp: number) =>
  bp >= 10000 ? null : Math.round((cost * 10000) / (10000 - bp));
