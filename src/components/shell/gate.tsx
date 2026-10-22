import {
  $,
  component$,
  Slot,
  useSignal,
  useStore,
  type QRL,
} from "@builder.io/qwik";
import { commands, type NewUser } from "~/lib/bindings";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { Alert } from "~/components/ui/ui";
import {
  btnPrimary,
  btnSecondary,
  card,
  hint,
  input,
  label,
} from "~/components/ui/styles";

// Screens shown before the app shell: first-time setup, the first admin,
// sign-in, and "this PC was removed".

export const Logo = component$<{ size?: "sm" | "lg" }>(({ size = "sm" }) => (
  <div
    class={[
      "inline-flex items-center rounded-full bg-brand-500 text-white",
      size === "lg" ? "gap-3 px-5 py-2" : "gap-2 px-3 py-1.5",
    ]}
  >
    <span
      class={[
        "grid place-items-center bg-white font-semibold text-brand-500",
        size === "lg" ? "size-8 text-lg" : "size-6 text-sm",
      ]}
    >
      S
    </span>
    <span class="leading-tight">
      <span class={["block", size === "lg" ? "text-xl" : "text-base"]}>
        Sanco
      </span>
      <span
        class={[
          "block font-semibold",
          size === "lg" ? "text-xs" : "text-[10px]",
        ]}
      >
        Medical Supply
      </span>
    </span>
  </div>
));

const Centered = component$(() => (
  <main class="grid min-h-screen place-items-center bg-surface-muted p-6">
    <Slot />
  </main>
));

export const Splash = component$(() => (
  <Centered>
    <div class="flex flex-col items-center gap-3 text-sm text-ink-muted">
      <Logo size="lg" />
      <p>Starting…</p>
    </div>
  </Centered>
));

/** Username, display name and password (twice) for a new account. */
const AccountFields = component$<{ user: NewUser; confirm: { value: string } }>(
  ({ user, confirm }) => (
    <>
      <label class="block space-y-1">
        <span class={label}>Your name</span>
        <input
          class={input}
          autoComplete="name"
          value={user.display_name}
          onInput$={(_, el) => (user.display_name = el.value)}
        />
      </label>
      <label class="block space-y-1">
        <span class={label}>Username</span>
        <input
          class={input}
          autoComplete="username"
          required
          value={user.username}
          onInput$={(_, el) => (user.username = el.value)}
        />
        <span class={hint}>Letters, digits, dots, dashes or underscores.</span>
      </label>
      <div class="grid grid-cols-2 gap-3">
        <label class="block space-y-1">
          <span class={label}>Password</span>
          <input
            class={input}
            type="password"
            autoComplete="new-password"
            required
            value={user.password}
            onInput$={(_, el) => (user.password = el.value)}
          />
        </label>
        <label class="block space-y-1">
          <span class={label}>Repeat password</span>
          <input
            class={input}
            type="password"
            autoComplete="new-password"
            required
            value={confirm.value}
            onInput$={(_, el) => (confirm.value = el.value)}
          />
        </label>
      </div>
    </>
  ),
);

export const Setup = component$<{ onDone$: QRL<() => void> }>(({ onDone$ }) => {
  const session = useSession();
  const ws = useSignal("Sanco Medical Supply");
  const admin = useStore<NewUser>({
    username: "",
    display_name: "",
    password: "",
    role_ids: [],
  });
  const confirm = useSignal("");
  const code = useSignal("");
  const error = useSignal("");
  const busy = useSignal<"" | "create" | "join" | "retry">("");
  const s = session.status;

  const run = $(
    async (kind: "create" | "join" | "retry", fn: () => Promise<unknown>) => {
      busy.value = kind;
      error.value = "";
      try {
        await fn();
        await onDone$();
      } catch (e) {
        error.value = errorText(e);
      } finally {
        busy.value = "";
      }
    },
  );

  return (
    <Centered>
      <div class="w-full max-w-4xl space-y-6">
        <div class="flex flex-col items-center gap-2 text-center">
          <Logo size="lg" />
          <h1 class="pt-2 text-xl font-semibold">Set up this PC</h1>
          <p class="text-sm text-ink-muted">
            Every PC keeps its own copy of the data and syncs with the others
            over Tailscale.
          </p>
        </div>

        {s?.error && (
          <Alert>
            <p>{errorText(s.error)}</p>
            <button
              class={[btnSecondary, "mt-2"]}
              disabled={!!busy.value}
              onClick$={() =>
                run("retry", async () => unwrap(await commands.retrySync()))
              }
            >
              {busy.value === "retry" ? "Retrying…" : "Retry"}
            </button>
          </Alert>
        )}
        {error.value && <Alert>{error.value}</Alert>}

        <div class="grid gap-6 md:grid-cols-2">
          <form
            class={[card, "space-y-3 p-5"]}
            preventdefault:submit
            onSubmit$={() => {
              if (admin.password !== confirm.value) {
                error.value = "The passwords don't match.";
                return;
              }
              return run("create", async () =>
                unwrap(await commands.createWorkspace(ws.value, { ...admin })),
              );
            }}
          >
            <h2 class="font-semibold">First PC: new workspace</h2>
            <p class={hint}>
              Do this once, on the first PC. You become its admin.
            </p>
            <label class="block space-y-1">
              <span class={label}>Company / workspace name</span>
              <input class={input} required bind:value={ws} />
            </label>
            <AccountFields user={admin} confirm={confirm} />
            <button class={btnPrimary} disabled={!!busy.value}>
              {busy.value === "create" ? "Creating…" : "Create workspace"}
            </button>
          </form>

          <form
            class={[card, "space-y-3 p-5"]}
            preventdefault:submit
            onSubmit$={() =>
              run("join", async () =>
                unwrap(await commands.joinWorkspace(code.value)),
              )
            }
          >
            <h2 class="font-semibold">Another PC: join</h2>
            <p class={hint}>
              On a PC that is already set up, an admin opens Settings → Devices
              &amp; sync → Show join code. Paste it here. Both PCs need
              Tailscale running.
            </p>
            <label class="block space-y-1">
              <span class={label}>Join code</span>
              <textarea
                class={[input, "h-28 font-mono text-xs"]}
                required
                bind:value={code}
              />
            </label>
            <button
              class={btnPrimary}
              disabled={!!busy.value || !code.value.trim()}
            >
              {busy.value === "join" ? "Joining…" : "Join workspace"}
            </button>
          </form>
        </div>
        {s && (
          <p class="text-center text-xs text-ink-muted">
            {s.pc_name} · data folder {s.data_dir} · sync port {s.port} · v
            {s.app_version}
          </p>
        )}
      </div>
    </Centered>
  );
});

export const CreateAdmin = component$<{ onDone$: QRL<() => void> }>(
  ({ onDone$ }) => {
    const admin = useStore<NewUser>({
      username: "",
      display_name: "",
      password: "",
      role_ids: [],
    });
    const confirm = useSignal("");
    const error = useSignal("");
    const busy = useSignal(false);
    return (
      <Centered>
        <form
          class={[card, "w-full max-w-md space-y-3 p-6"]}
          preventdefault:submit
          onSubmit$={async () => {
            if (admin.password !== confirm.value) {
              error.value = "The passwords don't match.";
              return;
            }
            busy.value = true;
            error.value = "";
            try {
              unwrap(await commands.createFirstAdmin({ ...admin }));
              await onDone$();
            } catch (e) {
              error.value = errorText(e);
            } finally {
              busy.value = false;
            }
          }}
        >
          <Logo />
          <h1 class="pt-2 text-lg font-semibold">Create the admin account</h1>
          <p class={hint}>
            This workspace has no users yet. The first account gets the Admin
            role.
          </p>
          {error.value && <Alert>{error.value}</Alert>}
          <AccountFields user={admin} confirm={confirm} />
          <button class={btnPrimary} disabled={busy.value}>
            Create and sign in
          </button>
        </form>
      </Centered>
    );
  },
);

export const Login = component$<{ onDone$: QRL<() => void> }>(({ onDone$ }) => {
  const session = useSession();
  const username = useSignal("");
  const password = useSignal("");
  const error = useSignal("");
  const busy = useSignal(false);
  const s = session.status;
  return (
    <Centered>
      <form
        class={[card, "w-full max-w-sm space-y-4 p-6"]}
        preventdefault:submit
        onSubmit$={async () => {
          busy.value = true;
          error.value = "";
          try {
            unwrap(await commands.login(username.value, password.value));
            password.value = "";
            await onDone$();
          } catch (e) {
            error.value = errorText(e);
          } finally {
            busy.value = false;
          }
        }}
      >
        <div class="flex justify-center">
          <Logo size="lg" />
        </div>
        <p class="text-center text-sm text-ink-muted">
          {s?.workspace} · {s?.pc_name}
        </p>
        {error.value && <Alert>{error.value}</Alert>}
        <label class="block space-y-1">
          <span class={label}>Username</span>
          <input
            class={input}
            autoComplete="username"
            autoFocus
            required
            bind:value={username}
          />
        </label>
        <label class="block space-y-1">
          <span class={label}>Password</span>
          <input
            class={input}
            type="password"
            autoComplete="current-password"
            required
            bind:value={password}
          />
        </label>
        <button class={[btnPrimary, "w-full"]} disabled={busy.value}>
          {busy.value ? "Signing in…" : "Sign in"}
        </button>
        <p class="text-center text-xs text-ink-muted">
          Forgot your password? Ask an admin to reset it.
        </p>
      </form>
    </Centered>
  );
});

export const Removed = component$<{ onDone$: QRL<() => void> }>(
  ({ onDone$ }) => {
    const error = useSignal("");
    const busy = useSignal(false);
    return (
      <Centered>
        <div class={[card, "w-full max-w-md space-y-3 p-6"]}>
          <Logo />
          <h1 class="pt-2 text-lg font-semibold">This PC was removed</h1>
          <p class="text-sm text-ink-muted">
            An admin removed this PC from the workspace, so it no longer syncs.
            To use it again, set it up again and join with a new join code. This
            PC's data is kept in its data folder, not deleted.
          </p>
          {error.value && <Alert>{error.value}</Alert>}
          <button
            class={btnPrimary}
            disabled={busy.value}
            onClick$={async () => {
              busy.value = true;
              try {
                unwrap(await commands.leaveWorkspace());
                await onDone$();
              } catch (e) {
                error.value = errorText(e);
              } finally {
                busy.value = false;
              }
            }}
          >
            Set up this PC again
          </button>
        </div>
      </Centered>
    );
  },
);
