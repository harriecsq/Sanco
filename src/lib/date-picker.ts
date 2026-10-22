// Themed calendar for every <input type="date">. WebView2's native picker
// is Windows blue and can't be styled, so the native one is hidden in
// global.css and this popover opens instead. The input stays a real date
// input (typing, min/max, form values); a pick sets its value and fires
// `input` + `change`, so the screens' existing handlers run unchanged.

const MONTHS = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];
const WEEKDAYS = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];

const navBtn =
  "flex h-7 w-7 items-center justify-center rounded-md text-brand-700 hover:bg-brand-50";
const dayBase =
  "flex h-8 w-8 items-center justify-center rounded-md text-sm transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-brand-300";
const linkBtn =
  "rounded-md px-2 py-1 text-xs font-medium text-brand-700 hover:bg-brand-50";

let pop: HTMLDivElement | null = null;
let target: HTMLInputElement | null = null;
let view = { y: 2000, m: 0 };
let installed = false;

const pad = (n: number) => String(n).padStart(2, "0");
const iso = (y: number, m: number, d: number) => `${y}-${pad(m + 1)}-${pad(d)}`;

/** Today in Asia/Manila as YYYY-MM-DD. */
function today(): string {
  return new Intl.DateTimeFormat("en-CA", { timeZone: "Asia/Manila" }).format(
    new Date(),
  );
}

function parse(v: string): { y: number; m: number; d: number } | null {
  const r = /^(\d{4})-(\d{2})-(\d{2})$/.exec(v);
  return r ? { y: +r[1], m: +r[2] - 1, d: +r[3] } : null;
}

function allowed(input: HTMLInputElement, v: string): boolean {
  return !((input.min && v < input.min) || (input.max && v > input.max));
}

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

function pick(v: string) {
  const input = target;
  if (!input) return;
  close(true);
  if (input.value === v) return;
  input.value = v;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

function render(focusDay?: string) {
  if (!pop || !target) return;
  const input = target;
  const { y, m } = view;
  const value = input.value;
  const now = today();
  pop.replaceChildren();

  const head = el("div", "mb-2 flex items-center justify-between gap-2");
  const prev = el("button", navBtn, "‹");
  prev.type = "button";
  prev.ariaLabel = "Previous month";
  prev.onclick = () => shift(-1);
  const next = el("button", navBtn, "›");
  next.type = "button";
  next.ariaLabel = "Next month";
  next.onclick = () => shift(1);
  head.append(
    prev,
    el("div", "text-sm font-semibold text-ink", `${MONTHS[m]} ${y}`),
    next,
  );

  const grid = el("div", "grid grid-cols-7 gap-0.5");
  grid.role = "grid";
  for (const w of WEEKDAYS) {
    grid.append(
      el(
        "div",
        "flex h-7 w-8 items-center justify-center text-xs font-medium text-ink-muted",
        w,
      ),
    );
  }
  const first = new Date(y, m, 1).getDay();
  const days = new Date(y, m + 1, 0).getDate();
  for (let i = 0; i < first; i++) grid.append(el("div", "h-8 w-8"));
  const sel = parse(value);
  const tabDay =
    focusDay && parse(focusDay)?.m === m
      ? focusDay
      : sel && sel.y === y && sel.m === m
        ? value
        : now.startsWith(`${y}-${pad(m + 1)}`)
          ? now
          : iso(y, m, 1);
  for (let d = 1; d <= days; d++) {
    const v = iso(y, m, d);
    const ok = allowed(input, v);
    const b = el("button", dayBase, String(d));
    b.type = "button";
    b.dataset.date = v;
    b.tabIndex = v === tabDay ? 0 : -1;
    if (v === value) {
      b.className += " bg-brand-500 font-semibold text-white hover:bg-brand-600";
      b.ariaSelected = "true";
    } else if (!ok) {
      b.className += " cursor-not-allowed text-ink-muted/40";
    } else if (v === now) {
      b.className += " font-semibold text-brand-700 ring-1 ring-brand-300 hover:bg-brand-50";
    } else {
      b.className += " text-ink hover:bg-brand-50 hover:text-brand-800";
    }
    if (ok) b.onclick = () => pick(v);
    else b.disabled = true;
    grid.append(b);
  }

  const foot = el(
    "div",
    "mt-2 flex items-center justify-between border-t border-line pt-2",
  );
  const clear = el("button", linkBtn, "Clear");
  clear.type = "button";
  clear.hidden = input.required;
  clear.onclick = () => pick("");
  const todayBtn = el("button", linkBtn, "Today");
  todayBtn.type = "button";
  todayBtn.disabled = !allowed(input, now);
  todayBtn.className += " disabled:opacity-40";
  todayBtn.onclick = () => pick(now);
  foot.append(clear, todayBtn);

  pop.append(head, grid, foot);
  if (focusDay) {
    pop.querySelector<HTMLButtonElement>(`[data-date="${tabDay}"]`)?.focus();
  }
}

function shift(by: number, focusDay?: string) {
  const d = new Date(view.y, view.m + by, 1);
  view = { y: d.getFullYear(), m: d.getMonth() };
  render(focusDay);
}

function place() {
  if (!pop || !target) return;
  const r = target.getBoundingClientRect();
  const w = pop.offsetWidth;
  const h = pop.offsetHeight;
  const below = r.bottom + 4 + h <= window.innerHeight;
  const top = below ? r.bottom + 4 : Math.max(4, r.top - 4 - h);
  const left = Math.min(Math.max(4, r.left), window.innerWidth - w - 4);
  pop.style.top = `${top}px`;
  pop.style.left = `${left}px`;
}

function open(input: HTMLInputElement, focus: boolean) {
  if (input.disabled || input.readOnly) return;
  if (target === input && pop?.matches(":popover-open")) return;
  close(false);
  target = input;
  const sel = parse(input.value) ?? parse(today())!;
  view = { y: sel.y, m: sel.m };

  pop = el(
    "div",
    "fixed inset-auto m-0 w-64 rounded-lg border border-line bg-white p-3 text-ink shadow-lg",
  );
  pop.popover = "manual";
  pop.role = "dialog";
  pop.ariaLabel = "Choose date";
  pop.onkeydown = onPopKey;
  // Inside a modal <dialog> everything outside it is inert, so the popover
  // has to live in the dialog.
  (input.closest("dialog") ?? document.body).append(pop);
  render();
  pop.showPopover();
  place();
  if (focus) {
    const day =
      pop.querySelector<HTMLButtonElement>('[aria-selected="true"]') ??
      pop.querySelector<HTMLButtonElement>('[tabindex="0"]');
    day?.focus();
  }
}

function close(refocus: boolean) {
  const input = target;
  pop?.remove();
  pop = null;
  target = null;
  if (refocus) input?.focus();
}

function onPopKey(e: KeyboardEvent) {
  if (e.key === "Escape") {
    // Keep a surrounding modal open.
    e.preventDefault();
    e.stopPropagation();
    close(true);
    return;
  }
  if (e.key === "Tab") {
    close(true);
    return;
  }
  const cur = (e.target as HTMLElement).dataset?.date;
  const p = cur && parse(cur);
  if (!p) return;
  const step: Record<string, number> = {
    ArrowLeft: -1,
    ArrowRight: 1,
    ArrowUp: -7,
    ArrowDown: 7,
  };
  let next: Date | null = null;
  if (e.key in step) next = new Date(p.y, p.m, p.d + step[e.key]);
  else if (e.key === "PageUp") next = new Date(p.y, p.m - 1, p.d);
  else if (e.key === "PageDown") next = new Date(p.y, p.m + 1, p.d);
  if (!next) return;
  e.preventDefault();
  const v = iso(next.getFullYear(), next.getMonth(), next.getDate());
  if (next.getFullYear() !== view.y || next.getMonth() !== view.m) {
    view = { y: next.getFullYear(), m: next.getMonth() };
    render(v);
  } else {
    pop?.querySelector<HTMLButtonElement>(`[data-date="${v}"]`)?.focus();
  }
}

function dateInput(t: EventTarget | null): HTMLInputElement | null {
  return t instanceof HTMLInputElement && t.type === "date" ? t : null;
}

/** Hooks every date input on the page. Call once, in the browser. */
export function installDatePicker() {
  if (installed) return;
  installed = true;

  document.addEventListener("click", (e) => {
    const input = dateInput(e.target);
    if (input) open(input, false);
  });
  document.addEventListener(
    "keydown",
    (e) => {
      const input = dateInput(e.target);
      if (!input) return;
      // Alt+Down and F4 would open the native picker.
      if ((e.altKey && e.key === "ArrowDown") || e.key === "F4") {
        e.preventDefault();
        open(input, true);
      } else if (e.key === "Escape" && target === input) {
        e.preventDefault();
        close(false);
      } else if (e.key === "Tab" && target === input) {
        close(false);
      }
    },
    true,
  );
  // Typing a date moves the calendar along.
  document.addEventListener("input", (e) => {
    const input = dateInput(e.target);
    const p = input && input === target && parse(input.value);
    if (!p) return;
    view = { y: p.y, m: p.m };
    render();
  });
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
