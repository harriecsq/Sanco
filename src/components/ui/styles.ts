// Shared Tailwind class sets, so screens stay consistent without a
// component for every element.

export const btn =
  "inline-flex items-center justify-center gap-1.5 rounded-md px-3 py-1.5 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50";
export const btnPrimary = `${btn} bg-brand-500 text-white hover:bg-brand-600`;
export const btnSecondary = `${btn} border border-line bg-white text-ink hover:bg-brand-50`;
export const btnDanger = `${btn} bg-danger text-white hover:bg-red-700`;
export const btnGhost = `${btn} text-brand-700 hover:bg-brand-50`;

export const inputBase =
  "rounded-md border border-line bg-white px-3 py-1.5 text-sm placeholder:text-ink-muted focus:border-brand-500 focus:outline-none focus:ring-2 focus:ring-brand-100";
export const input = `${inputBase} w-full`;
export const label = "block text-sm font-medium text-ink";
export const hint = "text-xs text-ink-muted";

export const card = "rounded-lg border border-line bg-white";

export const table = "w-full text-left text-sm";
export const th =
  "border-b border-line bg-surface-muted px-3 py-2 text-xs font-semibold uppercase tracking-wide text-ink-muted";
export const td = "border-b border-line px-3 py-2 align-top";
