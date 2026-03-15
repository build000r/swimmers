interface ClipboardDataLike {
  getData(type: string): string;
}

interface TerminalPasteEventLike {
  defaultPrevented: boolean;
  clipboardData?: ClipboardDataLike | null;
  preventDefault(): void;
}

interface HandleTerminalPasteEventOptions {
  observer: boolean;
  event: TerminalPasteEventLike;
  paste: (text: string) => void;
  notifyPasted: () => void;
}

export function handleTerminalPasteEvent({
  observer,
  event,
  paste,
  notifyPasted,
}: HandleTerminalPasteEventOptions): void {
  if (observer) {
    event.preventDefault();
    return;
  }

  // Avoid duplicate input when xterm/native paste already consumed this event.
  if (event.defaultPrevented) {
    return;
  }

  const text = event.clipboardData?.getData("text");
  if (!text) {
    return;
  }

  event.preventDefault();
  paste(text);
  notifyPasted();
}
