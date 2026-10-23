// Searchable list for long dropdowns (clients, suppliers, items, …). A
// <select> with more than SEARCH_FROM options opens this popover instead of
// its own list: a search box on top, the matching options below. The select
// stays a real select (value, form, required); a pick sets its value and
// fires `input` + `change`, so the screens' existing handlers run unchanged.
// Short lists (currency, status) keep the native themed list.

import { inputBase } from "~/components/ui/styles";

/** Lists longer than this get the search box. */
const SEARCH_FROM = 8;

const optBase =
  "flex w-full cursor-pointer items-center rounded-md px-2.5 py-1.5 text-left text-sm";

let pop: HTMLDivElement | null = null;
let search: HTMLInputElement | null = null;
let list: HTMLDivElement | null = null;
let target: HTMLSelectElement | null = null;
let shown: HTMLOptionElement[] = [];
let active = -1;
let installed = false;

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  cls: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

/** Lowercase without accents, so "pena" finds "Peña". */
function fold(s: string): string {
  return s.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
}

function off(o: HTMLOptionElement): boolean {
  const g = o.parentElement;
  return o.disabled || (g instanceof HTMLOptGroupElement && g.disabled);
}

function searchable(t: EventTarget | null): HTMLSelectElement | null {
  if (!(t instanceof HTMLSelectElement) || t.multiple || t.disabled) return null;
  if (t.dataset.search === "off") return null;
  return t.options.length >= SEARCH_FROM || t.dataset.search === "on" ? t : null;
}

function pick(o: HTMLOptionElement) {
  const sel = target;
  if (!sel || off(o)) return;
  close(true);
  if (sel.value === o.value && o.selected) return;
  sel.value = o.value;
  sel.dispatchEvent(new Event("input", { bubbles: true }));
  sel.dispatchEvent(new Event("change", { bubbles: true }));
}

function setActive(i: number, scroll: boolean) {
  if (!list) return;
  const rows = list.querySelectorAll<HTMLElement>("[data-i]");
  rows[active]?.removeAttribute("data-active");
  active = i;
  const row = rows[i];
  if (!row) return;
  row.setAttribute("data-active", "");
  search?.setAttribute("aria-activedescendant", row.id);
  if (scroll) row.scrollIntoView({ block: "nearest" });
}

function render() {
  if (!list || !target || !search) return;
  const sel = target;
  const terms = fold(search.value).split(/\s+/).filter(Boolean);
  list.replaceChildren();
  shown = [];

  let group: HTMLOptGroupElement | null = null;
  for (const o of Array.from(sel.options)) {
    if (o.hidden) continue;
    const text = o.label || o.text;
    // The "Choose…/All …" placeholder only shows before typing.
    if (terms.length && o.value === "") continue;
    if (terms.length && !terms.every((t) => fold(text).includes(t))) continue;

    const g = o.parentElement instanceof HTMLOptGroupElement ? o.parentElement : null;
    if (g && g !== group) {
      list.append(
        el("div", "px-2.5 pb-1 pt-2 text-xs font-semibold uppercase tracking-wide text-ink-muted", g.label),
      );
    }
    group = g;

    const i = shown.length;
    shown.push(o);
    const row = el("div", optBase, text);
    row.id = `select-search-opt-${i}`;
    row.role = "option";
    row.dataset.i = String(i);
    if (o.selected) {
      row.ariaSelected = "true";
      row.className += " bg-brand-500 font-medium text-white data-active:bg-brand-600";
    } else if (off(o)) {
      row.ariaDisabled = "true";
      row.className += " cursor-not-allowed text-ink-muted";
    } else {
      row.className += " text-ink hover:bg-brand-50 hover:text-brand-800 data-active:bg-brand-50 data-active:text-brand-800";
    }
    if (o.value === "") row.className += " italic";
    list.append(row);
  }

  if (!shown.length) {
    list.append(el("div", "px-2.5 py-3 text-center text-sm text-ink-muted", "No matches"));
  }
  const selIndex = shown.findIndex((o) => o.selected && o.value !== "");
  const first = shown.findIndex((o) => !off(o));
  active = -1;
  setActive(terms.length ? first : selIndex >= 0 ? selIndex : first, true);
}

function place() {
  if (!pop || !target) return;
  const r = target.getBoundingClientRect();
  pop.style.minWidth = `${Math.max(r.width, 224)}px`;
  const w = pop.offsetWidth;
  const h = pop.offsetHeight;
  const below = r.bottom + 4 + h <= window.innerHeight || r.top < h + 8;
  const top = below ? r.bottom + 4 : r.top - 4 - h;
  const left = Math.min(Math.max(4, r.left), window.innerWidth - w - 4);
  pop.style.top = `${Math.max(4, top)}px`;
  pop.style.left = `${left}px`;
}

function open(sel: HTMLSelectElement, query = "") {
  if (target === sel && pop) return;
  close(false);
  target = sel;
  sel.dataset.open = "";

  pop = el(
    "div",
    "fixed inset-auto m-0 flex max-w-md flex-col rounded-lg border border-line bg-white p-1 text-ink shadow-lg",
  );
  pop.popover = "manual";
  pop.onkeydown = onPopKey;

  search = el("input", `${inputBase} mb-1 w-full`);
  search.type = "search";
  search.placeholder = "Search…";
  search.autocomplete = "off";
  search.spellcheck = false;
  search.value = query;
  search.role = "combobox";
  search.ariaExpanded = "true";
  search.ariaAutoComplete = "list";
  search.ariaLabel = `Search ${sel.ariaLabel || sel.labels?.[0]?.textContent?.trim() || "options"}`;
  search.setAttribute("aria-controls", "select-search-list");
  search.oninput = () => render();

  list = el("div", "max-h-72 overflow-y-auto overscroll-contain");
  list.id = "select-search-list";
  list.role = "listbox";
  // mousedown, not click: keeps focus in the search box until the pick.
  list.onmousedown = (e) => {
    e.preventDefault();
    const row = (e.target as HTMLElement).closest<HTMLElement>("[data-i]");
    if (row) pick(shown[+row.dataset.i!]);
  };
  list.onmousemove = (e) => {
    const row = (e.target as HTMLElement).closest<HTMLElement>("[data-i]");
    if (row && +row.dataset.i! !== active) setActive(+row.dataset.i!, false);
  };

  pop.append(search, list);
  // Inside a modal <dialog> everything outside it is inert, so the popover
  // has to live in the dialog.
  (sel.closest("dialog") ?? document.body).append(pop);
  render();
  pop.showPopover();
  place();
  search.focus();
  search.setSelectionRange(query.length, query.length);
}

function close(refocus: boolean) {
  const sel = target;
  pop?.remove();
  pop = search = list = target = null;
  shown = [];
  active = -1;
  if (sel) delete sel.dataset.open;
  if (refocus) sel?.focus();
}

/** Moves the highlight `by` rows, past disabled ones, stopping at the ends. */
function move(by: number) {
  const step = Math.sign(by);
  let i = Math.min(Math.max(active + by, 0), shown.length - 1);
  while (i >= 0 && i < shown.length && off(shown[i])) i += step;
  if (i >= 0 && i < shown.length) setActive(i, true);
}

function onPopKey(e: KeyboardEvent) {
  const page = 8;
  switch (e.key) {
    case "Escape":
      // Keep a surrounding modal open.
      e.preventDefault();
      e.stopPropagation();
      close(true);
      return;
    case "Tab":
      close(true);
      return;
    case "Enter":
      e.preventDefault();
      if (shown[active]) pick(shown[active]);
      return;
    case "ArrowDown":
      e.preventDefault();
      move(1);
      return;
    case "ArrowUp":
      e.preventDefault();
      move(-1);
      return;
    case "PageDown":
      e.preventDefault();
      move(page);
      return;
    case "PageUp":
      e.preventDefault();
      move(-page);
      return;
  }
}

/** Hooks every long <select> on the page. Call once, in the browser. */
export function installSelectSearch() {
  if (installed) return;
  installed = true;

  // The native list opens on mousedown; open ours instead.
  document.addEventListener(
    "mousedown",
    (e) => {
      const sel = searchable(e.target);
      if (!sel || e.button !== 0) return;
      e.preventDefault();
      if (target === sel) {
        close(true);
      } else {
        sel.focus();
        open(sel);
      }
    },
    true,
  );
  document.addEventListener(
    "keydown",
    (e) => {
      const sel = searchable(e.target);
      if (!sel || e.ctrlKey || e.metaKey) return;
      const opens =
        e.key === " " ||
        e.key === "Enter" ||
        e.key === "F4" ||
        e.key === "ArrowDown" ||
        e.key === "ArrowUp";
      if (opens) {
        e.preventDefault();
        open(sel);
      } else if (e.key.length === 1 && !e.altKey) {
        // Typing on a closed dropdown starts a search.
        e.preventDefault();
        open(sel, e.key);
      }
    },
    true,
  );
  document.addEventListener("pointerdown", (e) => {
    if (!pop) return;
    const t = e.target as Node;
    if (!pop.contains(t) && t !== target) close(false);
  });
  document.addEventListener(
    "scroll",
    (e) => {
      if (pop && !pop.contains(e.target as Node)) close(false);
    },
    true,
  );
  window.addEventListener("resize", () => close(false));
  document.addEventListener("close", () => close(false), true);
}
