export function elementFromRef<T>(
  ref: T | { current?: T | null } | null | undefined,
): T | null | undefined;

export function assertStableIdentity<T extends Record<string, unknown>>(
  previous: T,
  next: T,
  options?: {
    keys?: string[];
    label?: string;
    noun?: string;
  },
): T;
