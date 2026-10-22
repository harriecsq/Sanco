import type { Action, Field, Module, Perms, SessionUser } from "./bindings";

export function can(
  user: SessionUser | null | undefined,
  module: Module,
  action: Action = "view",
): boolean {
  const p: Perms | undefined = user?.perms;
  if (!p) return false;
  return !!p.admin || !!p.modules?.[module]?.includes(action);
}

export function sees(user: SessionUser | null | undefined, field: Field) {
  const p = user?.perms;
  return !!p && (!!p.admin || !!p.fields?.includes(field));
}

export const isAdmin = (user: SessionUser | null | undefined) =>
  !!user?.perms.admin;
