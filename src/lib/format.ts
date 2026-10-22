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
