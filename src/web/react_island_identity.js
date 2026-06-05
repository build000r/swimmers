export function elementFromRef(ref) {
  return ref?.current ?? ref;
}

export function assertStableIdentity(previous, next, {
  keys = Object.keys(previous || {}),
  label = "React island",
  noun = "container",
} = {}) {
  for (const key of keys) {
    if (previous?.[key] !== next?.[key]) {
      throw new Error(`${label} replaced stable ${noun} ${key}`);
    }
  }
  return next;
}
