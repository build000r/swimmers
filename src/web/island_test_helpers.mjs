export function fakeElement(id) {
  return { id };
}

export function createElement(type, props, ...children) {
  return {
    type,
    props: { ...(props || {}), children },
  };
}

export function fakeDocumentForIds(ids) {
  const elements = new Map(Object.values(ids).map((id) => [id, fakeElement(id)]));
  return {
    documentRef: {
      getElementById(id) {
        return elements.get(id) ?? null;
      },
    },
    delete(id) {
      elements.delete(id);
    },
    remove(id) {
      elements.delete(id);
    },
    replace(id) {
      const replacement = { id, replaced: true };
      elements.set(id, replacement);
      return replacement;
    },
  };
}

export function keysFor(children) {
  return children.map((child) => child?.props?.key).filter(Boolean);
}

export function idsFor(children) {
  return children.map((child) => child?.props?.id).filter(Boolean);
}

export function buttonIdsFor(children) {
  return idsFor(children);
}
