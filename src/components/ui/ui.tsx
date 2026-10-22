import {
  component$,
  Slot,
  useSignal,
  useVisibleTask$,
  type Signal,
} from "@builder.io/qwik";

export const PageHeader = component$<{ title: string; subtitle?: string }>(
  ({ title, subtitle }) => (
    <header class="mb-5 flex flex-wrap items-end justify-between gap-3">
      <div>
        <h1 class="text-xl font-semibold text-ink">{title}</h1>
        {subtitle && <p class="mt-0.5 text-sm text-ink-muted">{subtitle}</p>}
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <Slot name="actions" />
      </div>
    </header>
  ),
);

type Tone = "brand" | "muted" | "danger" | "warning";

const tones: Record<Tone, string> = {
  brand: "bg-brand-50 text-brand-800",
  muted: "bg-surface-muted text-ink-muted border border-line",
  danger: "bg-red-50 text-danger",
  warning: "bg-amber-50 text-amber-800",
};

export const Badge = component$<{ tone?: Tone }>(({ tone = "muted" }) => (
  <span
    class={[
      "inline-flex items-center rounded px-2 py-0.5 text-xs font-medium",
      tones[tone],
    ]}
  >
    <Slot />
  </span>
));

export const Alert = component$<{ tone?: "danger" | "warning" | "brand" }>(
  ({ tone = "danger" }) => (
    <div
      role={tone === "danger" ? "alert" : "status"}
      class={[
        "rounded-md px-3 py-2 text-sm",
        tone === "danger" && "bg-red-50 text-danger",
        tone === "warning" && "bg-amber-50 text-amber-800",
        tone === "brand" && "bg-brand-50 text-brand-800",
      ]}
    >
      <Slot />
    </div>
  ),
);

export const Empty = component$(() => (
  <div class="px-3 py-10 text-center text-sm text-ink-muted">
    <Slot />
  </div>
));

/** A native `<dialog>`, open while `open` is true. Esc closes it. */
export const Modal = component$<{
  open: Signal<boolean>;
  title: string;
  wide?: boolean;
}>(({ open, title, wide }) => {
  const ref = useSignal<HTMLDialogElement>();
  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(
    ({ track }) => {
      const want = track(() => open.value);
      const d = ref.value;
      if (!d) return;
      if (want && !d.open) d.showModal();
      if (!want && d.open) d.close();
      // A closed dialog is never "visible", so don't wait for that.
    },
    { strategy: "document-ready" },
  );
  return (
    <dialog
      ref={ref}
      onClose$={() => (open.value = false)}
      class={[
        "m-auto w-full rounded-lg border border-line p-0 text-ink shadow-xl backdrop:bg-black/30",
        wide ? "max-w-3xl" : "max-w-md",
      ]}
    >
      <div class="flex items-center justify-between border-b border-line px-5 py-3">
        <h2 class="font-semibold">{title}</h2>
        <button
          type="button"
          aria-label="Close"
          class="rounded p-1 text-ink-muted hover:bg-surface-muted"
          onClick$={() => (open.value = false)}
        >
          ✕
        </button>
      </div>
      <div class="max-h-[75vh] overflow-y-auto px-5 py-4">
        <Slot />
      </div>
    </dialog>
  );
});

/** Shown instead of a screen the user may not open. */
export const NoAccess = component$(() => (
  <div class="rounded-lg border border-line bg-white px-6 py-12 text-center">
    <p class="font-medium">You don't have access to this screen.</p>
    <p class="mt-1 text-sm text-ink-muted">
      Ask an admin to give your role access.
    </p>
  </div>
));
