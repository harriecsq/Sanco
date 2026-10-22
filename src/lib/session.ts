import { createContextId, useContext } from "@builder.io/qwik";
import type { SessionUser, WorkspaceStatus } from "./bindings";

export interface SessionStore {
  status: WorkspaceStatus | null;
  user: SessionUser | null;
  /** Bumped when peers change, so sync indicators refetch. */
  syncTick: number;
}

export const SessionCtx = createContextId<SessionStore>("sanco.session");

export const useSession = () => useContext(SessionCtx);
