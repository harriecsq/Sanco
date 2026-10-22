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

/** Hundredths → input text: 123450 → "1234.50". */
export const moneyText = (v: number | null | undefined) =>
  v == null ? "" : (v / 100).toFixed(2);

/** Basis points → input text: 1250 → "12.5". */
export const pctText = (bp: number | null | undefined) =>
  bp == null ? "" : String(bp / 100);

/** Price at a margin on price, like the backend: cost ÷ (1 − margin). */
export const priceAtMargin = (cost: number, bp: number) =>
  bp >= 10000 ? null : Math.round((cost * 10000) / (10000 - bp));
