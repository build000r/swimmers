import { safeAnchorHref } from "./terminal_safety.js";

export function ensureDirBrowserBatchSelection(dirBrowser) {
  if (!(dirBrowser.batchSelected instanceof Set)) {
    dirBrowser.batchSelected = new Set();
  }
  return dirBrowser.batchSelected;
}

export function joinPath(base, name) {
  const root = String(base || "").replace(/\/+$/g, "");
  const child = String(name || "").replace(/^\/+/, "");
  if (!root) {
    return child ? `/${child}` : "/";
  }
  if (!child) {
    return root || "/";
  }
  if (root === "/") {
    return `/${child}`;
  }
  return `${root}/${child}`;
}

function escapeHtml(text) {
  return String(text || "").replace(/[&<>"']/g, (char) => {
    switch (char) {
      case "&":
        return "&amp;";
      case "<":
        return "&lt;";
      case ">":
        return "&gt;";
      case '"':
        return "&quot;";
      case "'":
        return "&#39;";
      default:
        return char;
    }
  });
}

export function dirEntryResolvedPath(basePath, entry) {
  const explicit = String(entry?.full_path || "").trim();
  if (explicit) {
    return explicit;
  }
  return joinPath(basePath, entry?.name || "");
}

export function dirEntryBatchSelectable(entry, resolvedPath) {
  if (!resolvedPath) {
    return false;
  }
  if (entry?.group && !entry?.full_path) {
    return false;
  }
  return true;
}

export function dirEntryGroups(entry) {
  const groups = Array.isArray(entry?.groups) ? entry.groups : [];
  const normalized = groups
    .map((group) => String(group || "").trim())
    .filter(Boolean);
  if (entry?.group && !normalized.includes(String(entry.group))) {
    normalized.push(String(entry.group));
  }
  return normalized;
}

function renderDirGroupActions(entry, entryPath, allGroups, activeGroup, readOnly = false) {
  const availableGroups = Array.isArray(allGroups) ? allGroups.map((group) => String(group || "").trim()).filter(Boolean) : [];
  if (!entryPath || !availableGroups.length) {
    return null;
  }
  const memberships = new Set(dirEntryGroups(entry));
  const wrap = document.createElement("div");
  wrap.className = "dir-row-group-actions";
  wrap.setAttribute("aria-label", `Group actions for ${entry?.name || entryPath}`);

  for (const groupName of availableGroups) {
    const isMember = memberships.has(groupName);
    const button = document.createElement("button");
    button.type = "button";
    button.className = `ghost-button dir-entry-group-action${isMember ? " is-member" : ""}`;
    button.dataset.path = entryPath;
    button.dataset.group = groupName;
    button.dataset.action = isMember ? "remove" : activeGroup && memberships.has(activeGroup) ? "move" : "add";
    if (button.dataset.action === "move") {
      button.dataset.removeGroup = activeGroup;
    }
    button.disabled = readOnly;
    button.textContent = isMember ? `remove ${groupName}` : button.dataset.action === "move" ? `move to ${groupName}` : `add ${groupName}`;
    wrap.appendChild(button);
  }

  return wrap;
}

export function normalizedDirSearch(dirBrowser) {
  return String(dirBrowser.search || "").trim().toLowerCase();
}

export function dirEntryMatchesSearch(entry, resolvedPath, search = "") {
  const normalizedSearch = String(search || "").trim().toLowerCase();
  if (!normalizedSearch) {
    return true;
  }
  const haystack = [
    entry?.name,
    resolvedPath,
    entry?.group,
    ...dirEntryGroups(entry),
    entry?.has_children ? "directory" : "leaf repo",
    entry?.is_running ? "running" : "",
    entry?.repo_dirty ? "dirty" : "",
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
  return haystack.includes(normalizedSearch);
}

export function visibleDirEntries(entries, basePath, search = "") {
  return entries.filter((entry) => dirEntryMatchesSearch(entry, dirEntryResolvedPath(basePath, entry), search));
}

export function visibleSelectableDirPaths(dirBrowser) {
  return visibleDirEntries(dirBrowser.entries, dirBrowser.path, normalizedDirSearch(dirBrowser))
    .map((entry) => [entry, dirEntryResolvedPath(dirBrowser.path, entry)])
    .filter(([entry, resolvedPath]) => dirEntryBatchSelectable(entry, resolvedPath))
    .map(([, resolvedPath]) => resolvedPath);
}

export function visibleDirBatchPlan(paths = [], currentPath = "", inputPath = "") {
  const selectedPaths = Array.isArray(paths) ? paths : [];
  return {
    paths: selectedPaths,
    firstPath: selectedPaths[0] || currentPath || inputPath,
    statusLabel: selectedPaths.length ? `Batching ${selectedPaths.length} visible directories.` : "No visible directories to batch.",
    statusMuted: selectedPaths.length < 1,
  };
}

export function selectedLaunchTarget(el, dirBrowser) {
  const value = String(el.createLaunchTarget?.value || dirBrowser.launchTarget || "local").trim();
  return value || "local";
}

export function launchTargetPayload(el, dirBrowser) {
  const target = selectedLaunchTarget(el, dirBrowser);
  return target && target !== "local" ? target : null;
}

function renderLaunchTargetOptions(response, { el, dirBrowser }) {
  if (!el.createLaunchTarget) {
    return;
  }
  const targets = Array.isArray(response?.launch_targets) && response.launch_targets.length
    ? response.launch_targets
    : [{ id: "local", label: "Local machine", kind: "local" }];
  const defaultTarget = String(response?.default_launch_target || dirBrowser.launchTarget || "local").trim() || "local";
  const hasDefault = targets.some((target) => String(target?.id || "") === defaultTarget);
  dirBrowser.launchTargets = targets;
  dirBrowser.launchTarget = hasDefault ? defaultTarget : String(targets[0]?.id || "local");
  el.createLaunchTarget.innerHTML = "";
  for (const target of targets) {
    const option = document.createElement("option");
    option.value = String(target?.id || "local");
    option.textContent = String(target?.label || target?.id || "Local machine");
    el.createLaunchTarget.appendChild(option);
  }
  el.createLaunchTarget.value = dirBrowser.launchTarget;
}

export function createRequestPreviewText(el) {
  const compact = String(el.createRequest?.value || "").replace(/\s+/g, " ").trim();
  if (!compact) {
    return "(none)";
  }
  if (compact.length > 72) {
    return `${compact.slice(0, 69)}...`;
  }
  return compact;
}

export function renderCreateBatchBar({ el, dirBrowser }) {
  const selected = ensureDirBrowserBatchSelection(dirBrowser);
  const count = selected.size;
  if (el.createBatchBar) {
    el.createBatchBar.classList.toggle("hidden", count < 1);
  }
  if (el.createBatchCount) {
    el.createBatchCount.textContent = `${count} selected`;
  }
  if (el.createBatchTool) {
    el.createBatchTool.textContent = `tool: ${String(el.createTool?.value || "grok").toLowerCase()} -> ${selectedLaunchTarget(el, dirBrowser)}`;
  }
  if (el.createBatchPreview) {
    el.createBatchPreview.textContent = `request: ${createRequestPreviewText(el)}`;
  }
}

export function clearCreateBatchSelection({ el, dirBrowser, syncSheetActionAvailability }) {
  const selected = ensureDirBrowserBatchSelection(dirBrowser);
  selected.clear();
  renderCreateBatchBar({ el, dirBrowser });
  syncSheetActionAvailability();
}

export function renderDirEntries(
  response,
  {
    el,
    dirBrowser,
    readOnly = false,
    storage = localStorage,
    pathStorageKey,
    managedOnlyStorageKey,
    setDirStatus,
    syncSheetActionAvailability,
  },
) {
  const rawEntries = Array.isArray(response?.entries) ? response.entries : [];
  const groups = Array.isArray(response?.groups) ? response.groups : [];
  const activeGroup = String(dirBrowser.group || "").trim();
  const selected = ensureDirBrowserBatchSelection(dirBrowser);
  const selectablePaths = new Set();

  dirBrowser.entries = rawEntries;
  dirBrowser.groups = groups;
  dirBrowser.overlayLabel = String(response?.overlay_label || "");
  const path = String(response?.path || el.createCwd.value || "").trim();
  dirBrowser.path = path;
  storage.setItem(pathStorageKey, path);
  storage.setItem(managedOnlyStorageKey, String(Boolean(el.dirsManagedOnly.checked)));
  el.dirsPath.value = path;
  if (!el.createCwd.value.trim() || !selected.size) {
    el.createCwd.value = path;
  }
  renderLaunchTargetOptions(response, { el, dirBrowser });
  const chipHost = el.dirsGroups || el.dirsList;
  if (el.dirsGroups) {
    el.dirsGroups.innerHTML = "";
  }
  el.dirsList.innerHTML = "";

  if (groups.length && chipHost) {
    const managed = Boolean(el.dirsManagedOnly.checked);
    const overlayLabel = String(response?.overlay_label || "managed").trim().toLowerCase();

    const managedButton = document.createElement("button");
    managedButton.type = "button";
    managedButton.className = "ghost-button dir-group-chip";
    managedButton.dataset.filter = "managed";
    managedButton.dataset.group = "";
    managedButton.textContent = overlayLabel || "managed";
    managedButton.classList.toggle("is-active", managed && !activeGroup);
    chipHost.appendChild(managedButton);

    const allButton = document.createElement("button");
    allButton.type = "button";
    allButton.className = "ghost-button dir-group-chip";
    allButton.dataset.filter = "all";
    allButton.dataset.group = "";
    allButton.textContent = "all folders";
    allButton.classList.toggle("is-active", !managed && !activeGroup);
    chipHost.appendChild(allButton);

    for (const groupName of groups) {
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = "ghost-button dir-group-chip";
      chip.dataset.filter = "group";
      chip.dataset.group = String(groupName || "");
      chip.textContent = String(groupName || "");
      chip.classList.toggle("is-active", chip.dataset.group === activeGroup);
      chipHost.appendChild(chip);
    }
  }

  const entries = visibleDirEntries(rawEntries, path, normalizedDirSearch(dirBrowser));

  if (!entries.length) {
    const empty = document.createElement("div");
    empty.className = "console-empty";
    empty.textContent = normalizedDirSearch(dirBrowser) ? "No directory matches." : "No child directories found.";
    el.dirsList.appendChild(empty);
  } else {
    for (const entry of entries) {
      const entryPath = dirEntryResolvedPath(path, entry);
      const selectable = dirEntryBatchSelectable(entry, entryPath);
      const row = document.createElement("div");
      row.className = "console-row dir-row";
      row.setAttribute("role", "row");
      row.dataset.path = entryPath;
      row.dataset.hasChildren = String(Boolean(entry.has_children));
      row.dataset.disabled = String(!selectable);
      if (entry?.group) {
        row.dataset.group = String(entry.group);
      }
      if (selectable) {
        selectablePaths.add(entryPath);
      }

      const running = Boolean(entry?.is_running);
      const dirty = Boolean(entry?.repo_dirty);
      const memberships = dirEntryGroups(entry);
      const managed = memberships.length > 0 || Boolean(entry?.group);
      const managedTitle = memberships.length ? `groups: ${memberships.join(", ")}` : "managed repository";

      const selectCell = document.createElement("div");
      selectCell.className = "col-select dir-select-cell";
      const checkbox = document.createElement("input");
      checkbox.type = "checkbox";
      checkbox.className = "dir-row-check";
      checkbox.dataset.path = entryPath;
      checkbox.disabled = readOnly || !selectable;
      checkbox.checked = selectable && selected.has(entryPath);
      checkbox.setAttribute("aria-label", `Include ${entry.name} in batch send`);
      selectCell.appendChild(checkbox);
      row.appendChild(selectCell);

      const main = document.createElement("button");
      main.type = "button";
      main.className = "col-name dir-row-main";
      main.dataset.path = entryPath;
      main.dataset.hasChildren = String(Boolean(entry.has_children));
      if (entry?.group) {
        main.dataset.group = String(entry.group);
      }
      main.title = entryPath;
      main.tabIndex = -1;
      if (!entryPath) {
        main.disabled = true;
      }
      main.innerHTML = `
        <span class="dir-row-kind ${entry.has_children ? "is-dir" : "is-repo"}" aria-hidden="true">${entry.has_children ? "▸" : "◆"}</span>
        <span class="dir-row-name">${escapeHtml(entry.name || "(unnamed)")}</span>
      `;
      row.appendChild(main);

      const pathCell = document.createElement("span");
      pathCell.className = "col-path dir-row-path";
      pathCell.title = entryPath;
      pathCell.textContent = entryPath || "(no path)";
      row.appendChild(pathCell);

      const statusCell = document.createElement("div");
      statusCell.className = "col-status dir-row-status";
      const managedBadge = document.createElement("span");
      managedBadge.className = `dir-badge ${managed ? "is-managed" : "is-unmanaged"}`;
      managedBadge.textContent = managed ? "managed" : "local";
      managedBadge.title = managed ? managedTitle : "not in a managed group";
      statusCell.appendChild(managedBadge);
      if (running) {
        const runningBadge = document.createElement("span");
        runningBadge.className = "dir-badge is-running";
        runningBadge.textContent = "running";
        statusCell.appendChild(runningBadge);
      }
      if (dirty) {
        const dirtyBadge = document.createElement("span");
        dirtyBadge.className = "dir-badge is-dirty";
        dirtyBadge.textContent = "dirty";
        statusCell.appendChild(dirtyBadge);
      }
      row.appendChild(statusCell);

      const groupsCell = document.createElement("div");
      groupsCell.className = "col-groups dir-row-groups";
      const openHref = safeAnchorHref(entry?.open_url);
      if (openHref) {
        const openLink = document.createElement("a");
        openLink.className = "dir-open-url";
        openLink.href = openHref;
        openLink.target = "_blank";
        openLink.rel = "noopener noreferrer";
        openLink.textContent = "open url";
        groupsCell.appendChild(openLink);
      }
      const groupActions = renderDirGroupActions(entry, entryPath, groups, activeGroup, readOnly);
      if (groupActions) {
        groupsCell.appendChild(groupActions);
      }
      row.appendChild(groupsCell);
      el.dirsList.appendChild(row);
    }
  }

  for (const selectedPath of Array.from(selected)) {
    if (!selectablePaths.has(selectedPath)) {
      selected.delete(selectedPath);
    }
  }

  const managed = Boolean(el.dirsManagedOnly.checked);
  const shownCount = entries.length;
  const totalCount = rawEntries.length;
  const searchSuffix = normalizedDirSearch(dirBrowser) ? ` · ${shownCount}/${totalCount} search matches` : "";
  const targetSuffix = selectedLaunchTarget(el, dirBrowser) !== "local" ? ` · target ${selectedLaunchTarget(el, dirBrowser)}` : "";
  const summary = response?.path
    ? `${shownCount} entries at ${response.path}${managed ? " (managed only)" : ""}${activeGroup ? ` · group ${activeGroup}` : ""}${searchSuffix}${targetSuffix}`
    : "Select a directory to continue.";
  setDirStatus(summary);
  renderCreateBatchBar({ el, dirBrowser });
  syncSheetActionAvailability();
}
