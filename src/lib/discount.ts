// Invoice discounts given on payments: a percentage of the invoice's total
// or a set peso amount. Same rounding as the backend (`pricing::Discount`).
import type { Discount } from "~/lib/bindings";
import { formatPct, moneyText, parseMoney, parsePct, pctText } from "~/lib/format";

export type DiscountMode = Discount["kind"];

/** A stored discount as input text: 500 bp → "5", ₱50 → "50.00". */
export const discountText = (d: Discount | null | undefined) =>
  d == null ? "" : d.kind === "percent" ? pctText(d.value) : moneyText(d.value);

/** The typed discount; blank or 0 is none. Throws on bad input. */
export function parseDiscount(mode: DiscountMode, text: string): Discount | null {
  const value =
    mode === "percent"
      ? parsePct(text, "discount percentage")
      : parseMoney(text, "discount amount");
  return value ? { kind: mode, value } : null;
}

/** What a discount takes off an invoice of `total`: rounded half away
 *  from zero, never more than the total. */
export function discountOf(d: Discount | null | undefined, total: number): number {
  if (!d || total <= 0) return 0;
  const off = d.kind === "percent" ? Math.round((total * d.value) / 10000) : d.value;
  return Math.min(Math.max(off, 0), total);
}

/** What the typed discount takes off; 0 while blank or not valid. */
export function typedDiscountOf(mode: DiscountMode, text: string, total: number): number {
  try {
    return discountOf(parseDiscount(mode, text), total);
  } catch {
    return 0;
  }
}

/** "5%" or "Set amount". */
export const discountAs = (d: Discount | null | undefined) =>
  d?.kind === "percent" ? formatPct(d.value) : d ? "Set amount" : "";
