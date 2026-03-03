export type TerminalShortcutAction =
  | "copy"
  | "native_paste"
  | "block_paste"
  | "none";

interface ShortcutEventShape {
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  key: string;
}

export function resolveTerminalShortcutAction(
  event: ShortcutEventShape,
  observer: boolean,
): TerminalShortcutAction {
  const isAccel = (event.metaKey || event.ctrlKey) && !event.altKey;
  if (!isAccel) return "none";

  const key = event.key.toLowerCase();
  if (key === "c") return "copy";
  if (key === "v") return observer ? "block_paste" : "native_paste";
  return "none";
}
