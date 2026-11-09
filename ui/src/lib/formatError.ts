/**
 * Shared error formatting — `String(e)` on a structured `{code,
 * message}` envelope renders "[object Object]" (goal §2a). Every UI
 * catch site routes through `formatError()` here.
 */

export function formatError(value: unknown): string {
  if (value instanceof Error) {
    const code = (value as unknown as { code?: string }).code;
    const base = code ? `${code}: ${value.message}` : value.message;
    return base || "Unknown error";
  }
  if (value && typeof value === "object") {
    const e = value as { code?: unknown; message?: unknown };
    if (typeof e.message === "string" && typeof e.code === "string") {
      return `${e.code}: ${e.message}`;
    }
    if (typeof e.message === "string") return e.message;
  }
  if (typeof value === "string") return value;
  return value == null ? "Unknown error" : JSON.stringify(value);
}

/** Test hook: fails if anything renders as the literal "[object Object]". */
export function plainErrorForTest(value: unknown): string {
  const rendered = formatError(value);
  if (rendered === "[object Object]") {
    throw new Error("formatError must never return [object Object]");
  }
  return rendered;
}
