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

export function renderDirGroupActionPlan(entry, entryPath, allGroups, activeGroup) {
  const availableGroups = Array.isArray(allGroups) ? allGroups.map((group) => String(group || "").trim()).filter(Boolean) : [];
  if (!entryPath || !availableGroups.length) {
    return [];
  }
  const memberships = new Set(dirEntryGroups(entry));
  return availableGroups.map((groupName) => {
    const isMember = memberships.has(groupName);
    const action = isMember ? "remove" : activeGroup && memberships.has(activeGroup) ? "move" : "add";
    return {
      groupName,
      isMember,
      action,
      removeGroup: action === "move" ? activeGroup : "",
      label: isMember ? `remove ${groupName}` : action === "move" ? `move to ${groupName}` : `add ${groupName}`,
    };
  });
}

function renderDirGroupActions(entry, entryPath, allGroups, activeGroup, readOnly = false) {
  const actions = renderDirGroupActionPlan(entry, entryPath, allGroups, activeGroup);
  if (!actions.length) {
    return null;
  }
  const wrap = document.createElement("div");
  wrap.className = "dir-row-group-actions";
  wrap.setAttribute("aria-label", `Group actions for ${entry?.name || entryPath}`);

  for (const { groupName, isMember, action, removeGroup, label } of actions) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = `ghost-button dir-entry-group-action${isMember ? " is-member" : ""}`;
    button.dataset.path = entryPath;
    button.dataset.group = groupName;
    button.dataset.action = action;
    if (action === "move") {
      button.dataset.removeGroup = removeGroup;
    }
    button.disabled = readOnly;
    button.textContent = label;
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
  const terms = normalizedSearch.split(/\s+/).filter(Boolean);
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
  return terms.every((term) => haystack.includes(term));
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

export function dirCheckboxChangePlan(eventType, target) {
  if (eventType !== "change") {
    return { type: "ignore" };
  }
  const checkbox = target?.closest?.(".dir-row-check") ?? null;
  if (!checkbox) {
    return { type: "ignore" };
  }
  const path = String(checkbox.dataset.path || "").trim();
  if (!path) {
    return { type: "reset_checkbox", checkbox };
  }
  return checkbox.checked ? { type: "add", path } : { type: "remove", path };
}

export function dirGroupChipClickPlan(eventType, target, managedOnlyChecked = false, currentPath = "", inputPath = "") {
  if (eventType !== "click") {
    return { type: "ignore" };
  }
  const button = target?.closest?.(".dir-group-chip") ?? null;
  if (!button) {
    return { type: "ignore" };
  }
  const filter = String(button.dataset.filter || "group");
  const groupName = String(button.dataset.group || "").trim();
  const managedOnly = filter === "managed" ? true : filter === "all" ? false : Boolean(managedOnlyChecked);
  return { type: "filter", group: filter === "group" ? groupName : "", managedOnly, path: currentPath || inputPath };
}

export function dirGroupMembershipClickPlan(eventType, target) {
  if (eventType !== "click") {
    return { type: "ignore" };
  }
  const button = target?.closest?.(".dir-entry-group-action") ?? null;
  if (!button) {
    return { type: "ignore" };
  }
  const dataset = button.dataset || {};
  return { type: "membership", path: dataset.path, action: dataset.action, group: dataset.group, removeGroup: dataset.removeGroup };
}

export function dirRowClickPlan(eventType, target) {
  if (eventType !== "click") {
    return { type: "ignore" };
  }
  const row = target?.closest?.(".dir-row-main") ?? null;
  if (!row) {
    return { type: "ignore" };
  }
  const path = String(row.dataset.path || "").trim();
  if (!path) {
    return { type: "ignore" };
  }
  return { type: "row", path, hasChildren: row.dataset.hasChildren === "true" };
}

export function selectedLaunchTarget(el, dirBrowser) {
  const value = String(el.createLaunchTarget?.value || dirBrowser.launchTarget || "local").trim();
  return value || "local";
}

export function launchTargetPayload(el, dirBrowser) {
  const target = selectedLaunchTarget(el, dirBrowser);
  return target && target !== "local" ? target : null;
}

function localLaunchTarget() {
  return { id: "local", label: "Local machine", kind: "local", path_mappings: [] };
}

function normalizeLaunchTarget(target) {
  const id = String(target?.id || "").trim();
  if (!id) {
    return null;
  }
  if (id === "local") {
    return localLaunchTarget();
  }
  const label = String(target?.label || id).trim() || id;
  const kind = normalizeLaunchTargetKind(target?.kind);
  return {
    ...target,
    id,
    label,
    kind,
    path_mappings: Array.isArray(target?.path_mappings) ? target.path_mappings : [],
  };
}

function normalizeLaunchTargetKind(kind) {
  const normalized = String(kind || "").trim().toLowerCase();
  return normalized === "ssh" ? "ssh_only" : normalized;
}

export function normalizeLaunchTargets(targets) {
  const normalized = (Array.isArray(targets) ? targets : [])
    .map((target) => normalizeLaunchTarget(target))
    .filter(Boolean);
  const hasLocal = normalized.some((target) => String(target?.id || "") === "local");
  return hasLocal ? normalized : [localLaunchTarget(), ...normalized];
}

export function launchTargetById(dirBrowser, targetId) {
  const normalizedId = String(targetId || "local").trim() || "local";
  const targets = Array.isArray(dirBrowser?.launchTargets) ? dirBrowser.launchTargets : [];
  return targets.find((target) => String(target?.id || "").trim() === normalizedId)
    || (normalizedId === "local" ? localLaunchTarget() : null);
}

export function selectedLaunchTargetSummary(el, dirBrowser) {
  return launchTargetById(dirBrowser, selectedLaunchTarget(el, dirBrowser))
    || { id: selectedLaunchTarget(el, dirBrowser), label: selectedLaunchTarget(el, dirBrowser), kind: "", path_mappings: [] };
}

function normalizeLaunchPath(path) {
  const raw = String(path || "").trim();
  const absolute = raw.startsWith("/");
  const parts = [];
  for (const part of raw.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      parts.pop();
    } else {
      parts.push(part);
    }
  }
  const joined = parts.join("/");
  return absolute ? `/${joined}` || "/" : joined || ".";
}

function relativeLaunchPath(path, prefix) {
  const normalizedPath = normalizeLaunchPath(path);
  const normalizedPrefix = normalizeLaunchPath(prefix).replace(/\/+$/g, "") || "/";
  if (normalizedPath === normalizedPrefix) {
    return "";
  }
  const prefixWithSlash = normalizedPrefix === "/" ? "/" : `${normalizedPrefix}/`;
  return normalizedPath.startsWith(prefixWithSlash) ? normalizedPath.slice(prefixWithSlash.length) : null;
}

function joinLaunchPath(prefix, relative) {
  const normalizedPrefix = normalizeLaunchPath(prefix).replace(/\/+$/g, "") || "/";
  const normalizedRelative = String(relative || "").replace(/^\/+/, "");
  if (!normalizedRelative) {
    return normalizedPrefix;
  }
  return normalizedPrefix === "/" ? `/${normalizedRelative}` : `${normalizedPrefix}/${normalizedRelative}`;
}

export function mapPathWithLaunchTarget(path, target) {
  const mappings = Array.isArray(target?.path_mappings) ? target.path_mappings : [];
  let best = null;
  for (const mapping of mappings) {
    const localPrefix = String(mapping?.local_prefix || "").trim();
    const remotePrefix = String(mapping?.remote_prefix || "").trim();
    if (!localPrefix || !remotePrefix) {
      continue;
    }
    const relative = relativeLaunchPath(path, localPrefix);
    if (relative === null) continue;
    const score = normalizeLaunchPath(localPrefix).split("/").filter(Boolean).length;
    if (!best || score > best.score) {
      best = { score, path: joinLaunchPath(remotePrefix, relative) };
    }
  }
  return best?.path || null;
}

export function launchTargetPreviewForPath(path, target) {
  const targetId = String(target?.id || "local").trim() || "local";
  const targetLabel = String(target?.label || targetId || "Local machine").trim() || targetId;
  const targetKind = String(target?.kind || "").trim().toLowerCase();
  const localCwd = String(path || "").trim();
  if (targetId === "local") {
    return { targetId, targetLabel, localCwd, remoteCwd: null, blocked: false, reason: "" };
  }
  if (targetKind === "ssh_only" || targetKind === "ssh") {
    return { targetId, targetLabel, localCwd, remoteCwd: null, blocked: false, reason: "handoff" };
  }
  const configBlocker = launchTargetConfigBlocker(target);
  if (configBlocker) {
    return { targetId, targetLabel, localCwd, remoteCwd: null, blocked: true, reason: configBlocker };
  }
  const remoteCwd = mapPathWithLaunchTarget(localCwd, target);
  return remoteCwd
    ? { targetId, targetLabel, localCwd, remoteCwd, blocked: false, reason: "" }
    : { targetId, targetLabel, localCwd, remoteCwd: null, blocked: true, reason: "unmapped cwd" };
}

function launchTargetConfigBlocker(target) {
  if (normalizeLaunchTargetKind(target?.kind) !== "swimmers_api") {
    return "unsupported target";
  }
  const baseUrl = String(target?.base_url || "").trim();
  if (!baseUrl) {
    return "missing base_url";
  }
  try {
    const parsed = new URL(baseUrl);
    if (!parsed.hostname || !["http:", "https:"].includes(parsed.protocol)) {
      return "invalid base_url";
    }
    if (parsed.username || parsed.password || parsed.search || parsed.hash) {
      return "invalid base_url";
    }
  } catch {
    return "invalid base_url";
  }
  return "";
}

export function launchTargetPreviewText(path, target) {
  const preview = launchTargetPreviewForPath(path, target);
  if (preview.targetId === "local") {
    return `local: ${preview.localCwd || "(no cwd)"}`;
  }
  if (preview.blocked) {
    return `${preview.targetLabel}: ${preview.reason}`;
  }
  if (preview.reason === "handoff") {
    return `${preview.targetLabel}: handoff for ${preview.localCwd || "(no cwd)"}`;
  }
  return `${preview.targetLabel}: ${preview.remoteCwd}`;
}

export function launchTargetStatusTextForPreview(preview) {
  if (!preview) {
    return "";
  }
  if (preview.targetId === "local") {
    return `Launch target local: ${preview.localCwd || "(no cwd)"}`;
  }
  if (preview.blocked) {
    return `${preview.targetLabel}: ${preview.reason} for ${preview.localCwd || "(no cwd)"}`;
  }
  if (preview.reason === "handoff") {
    return `${preview.targetLabel}: handoff for ${preview.localCwd || "(no cwd)"}`;
  }
  return `${preview.targetLabel}: ${preview.localCwd || "(no cwd)"} -> ${preview.remoteCwd}`;
}

export function launchTargetBlockersForPaths(paths = [], target) {
  return (Array.isArray(paths) ? paths : [])
    .map((path) => launchTargetPreviewForPath(path, target))
    .filter((preview) => preview.blocked);
}

export function batchLaunchTargetBlockers(dirBrowser, target) {
  const selected = ensureDirBrowserBatchSelection(dirBrowser);
  return launchTargetBlockersForPaths(Array.from(selected), target);
}

function renderLaunchTargetOptions(response, { el, dirBrowser, preferredLaunchTarget = "" }) {
  if (!el.createLaunchTarget) {
    return;
  }
  const targets = normalizeLaunchTargets(response?.launch_targets);
  const preferredTarget = String(preferredLaunchTarget || "").trim();
  const defaultTarget = String(preferredTarget || response?.default_launch_target || dirBrowser.launchTarget || "local").trim() || "local";
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
  const target = selectedLaunchTargetSummary(el, dirBrowser);
  const blockers = batchLaunchTargetBlockers(dirBrowser, target);
  dirBrowser.batchLaunchBlockers = blockers;
  if (el.createBatchBar) {
    el.createBatchBar.classList.toggle("hidden", count < 1);
  }
  if (el.createBatchCount) {
    el.createBatchCount.textContent = `${count} selected`;
  }
  if (el.createBatchTool) {
    const targetLabel = String(target?.label || target?.id || "local");
    const blockedSuffix = blockers.length ? ` (${blockers.length} unmapped)` : "";
    el.createBatchTool.textContent = `tool: ${String(el.createTool?.value || "grok").toLowerCase()} -> ${targetLabel}${blockedSuffix}`;
  }
  if (el.createBatchPreview) {
    const firstPath = selected.values().next().value || el.createCwd?.value || "";
    const targetPreview = count > 1 && !blockers.length && String(target?.id || "local") !== "local"
      ? String(target?.kind || "").trim().toLowerCase() === "ssh_only"
        ? `${target.label}: ${count} handoff`
        : `${target.label}: ${count} mapped`
      : launchTargetPreviewText(firstPath, target);
    el.createBatchPreview.textContent = `target: ${targetPreview} · request: ${createRequestPreviewText(el)}`;
  }
}

export function clearCreateBatchSelection({ el, dirBrowser, syncSheetActionAvailability }) {
  const selected = ensureDirBrowserBatchSelection(dirBrowser);
  selected.clear();
  renderCreateBatchBar({ el, dirBrowser });
  syncSheetActionAvailability();
}

function dirBrowserSelectablePathSet(entries, path) {
  return new Set(
    entries
      .map((entry) => [entry, dirEntryResolvedPath(path, entry)])
      .filter(([entry, entryPath]) => dirEntryBatchSelectable(entry, entryPath))
      .map(([, entryPath]) => entryPath),
  );
}

function renderLegacyDirBrowserView(view, { el }) {
  const chipHost = el.dirsGroups || el.dirsList;
  if (el.dirsGroups) {
    el.dirsGroups.innerHTML = "";
  }
  el.dirsList.innerHTML = "";

  if (view.groups.length && chipHost) {
    const managedButton = document.createElement("button");
    managedButton.type = "button";
    managedButton.className = "ghost-button dir-group-chip";
    managedButton.dataset.filter = "managed";
    managedButton.dataset.group = "";
    managedButton.textContent = view.overlayLabel || "managed";
    managedButton.classList.toggle("is-active", view.managed && !view.activeGroup);
    chipHost.appendChild(managedButton);

    const allButton = document.createElement("button");
    allButton.type = "button";
    allButton.className = "ghost-button dir-group-chip";
    allButton.dataset.filter = "all";
    allButton.dataset.group = "";
    allButton.textContent = "all folders";
    allButton.classList.toggle("is-active", !view.managed && !view.activeGroup);
    chipHost.appendChild(allButton);

    for (const groupName of view.groups) {
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = "ghost-button dir-group-chip";
      chip.dataset.filter = "group";
      chip.dataset.group = String(groupName || "");
      chip.textContent = String(groupName || "");
      chip.classList.toggle("is-active", chip.dataset.group === view.activeGroup);
      chipHost.appendChild(chip);
    }
  }

  if (!view.entries.length) {
    const empty = document.createElement("div");
    empty.className = "console-empty";
    empty.textContent = view.search ? "No directory matches." : "No child directories found.";
    el.dirsList.appendChild(empty);
  } else {
    for (const entry of view.entries) {
      const entryPath = dirEntryResolvedPath(view.path, entry);
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
      checkbox.disabled = view.readOnly || !selectable;
      checkbox.checked = selectable && view.selectedPaths.has(entryPath);
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
      const groupActions = renderDirGroupActions(
        entry,
        entryPath,
        view.groups,
        view.activeGroup,
        view.groupActionsReadOnly ?? view.readOnly,
      );
      if (groupActions) {
        groupsCell.appendChild(groupActions);
      }
      row.appendChild(groupsCell);
      el.dirsList.appendChild(row);
    }
  }
}

export function renderDirEntries(
  response,
  {
    el,
    dirBrowser,
    readOnly = false,
    groupActionsReadOnly = readOnly,
    storage = localStorage,
    pathStorageKey,
    managedOnlyStorageKey,
    setDirStatus,
    syncSheetActionAvailability,
    renderDirBrowserView = null,
    preferredLaunchTarget = "",
  },
) {
  const rawEntries = Array.isArray(response?.entries) ? response.entries : [];
  const groups = Array.isArray(response?.groups) ? response.groups : [];
  const activeGroup = String(dirBrowser.group || "").trim();
  const selected = ensureDirBrowserBatchSelection(dirBrowser);

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
  renderLaunchTargetOptions(response, { el, dirBrowser, preferredLaunchTarget });

  const entries = visibleDirEntries(rawEntries, path, normalizedDirSearch(dirBrowser));
  const selectablePaths = dirBrowserSelectablePathSet(entries, path);
  const managed = Boolean(el.dirsManagedOnly.checked);
  const view = {
    groups,
    entries,
    path,
    activeGroup,
    selectedPaths: selected,
    readOnly,
    groupActionsReadOnly,
    managed,
    overlayLabel: String(response?.overlay_label || "managed").trim().toLowerCase(),
    search: normalizedDirSearch(dirBrowser),
  };

  const renderedByIsland = typeof renderDirBrowserView === "function"
    && renderDirBrowserView(view) === true;
  if (!renderedByIsland) {
    renderLegacyDirBrowserView(view, { el });
  }

  for (const selectedPath of Array.from(selected)) {
    if (!selectablePaths.has(selectedPath)) {
      selected.delete(selectedPath);
    }
  }

  const shownCount = entries.length;
  const totalCount = rawEntries.length;
  const searchSuffix = view.search ? ` · ${shownCount}/${totalCount} search matches` : "";
  const targetSuffix = selectedLaunchTarget(el, dirBrowser) !== "local" ? ` · target ${selectedLaunchTarget(el, dirBrowser)}` : "";
  const summary = response?.path
    ? `${shownCount} entries at ${response.path}${managed ? " (managed only)" : ""}${activeGroup ? ` · group ${activeGroup}` : ""}${searchSuffix}${targetSuffix}`
    : "Select a directory to continue.";
  setDirStatus(summary);
  renderCreateBatchBar({ el, dirBrowser });
  syncSheetActionAvailability();
}
