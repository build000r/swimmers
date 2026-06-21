export function elementFromRef<T>(
  ref: T | { current?: T | null } | null | undefined,
): T | null | undefined;

export interface IdentityDriftDetail {
  message: string;
  label?: string;
  noun?: string;
  key?: string;
}

export function setIdentityDriftReporter(
  reporter: ((detail: IdentityDriftDetail) => void) | null,
): ((detail: IdentityDriftDetail) => void) | null;

export function reportIdentityDrift(
  message: string,
  details?: Omit<IdentityDriftDetail, "message">,
): void;

export function assertStableIdentity<T extends Record<string, unknown>>(
  previous: T,
  next: T,
  options?: {
    keys?: string[];
    label?: string;
    noun?: string;
    throwOnDrift?: boolean;
  },
): T;
