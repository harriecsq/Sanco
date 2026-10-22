// Commands return `{ status, data | error }`. These turn that into
// exceptions so screens can use one try/catch.

export type CmdResult<T> =
  | { status: "ok"; data: T }
  | { status: "error"; error: string };

export class CommandError extends Error {}

export function unwrap<T>(res: CmdResult<T>): T {
  if (res.status === "error") throw new CommandError(res.error);
  return res.data;
}

/** First letter upper-cased: backend messages are lower-case fragments. */
export function errorText(e: unknown): string {
  const msg = e instanceof Error ? e.message : String(e);
  return msg.charAt(0).toUpperCase() + msg.slice(1);
}
