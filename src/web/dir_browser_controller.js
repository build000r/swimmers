import {
  clearCreateBatchSelection as clearDirBrowserBatchSelection,
  dirCheckboxChangePlan,
  dirGroupChipClickPlan,
  dirGroupMembershipClickPlan,
  dirRowClickPlan,
  ensureDirBrowserBatchSelection as ensureDirBrowserBatchSelectionState,
  launchTargetBlockersForPaths as dirBrowserLaunchTargetBlockersForPaths,
  launchTargetById as dirBrowserLaunchTargetById,
  launchTargetPayload as dirBrowserLaunchTargetPayload,
  launchTargetPreviewForPath as dirBrowserLaunchTargetPreviewForPath,
  launchTargetStatusTextForPreview as dirBrowserLaunchTargetStatusTextForPreview,
  normalizedDirGroupList,
  renderCreateBatchBar as renderDirBrowserCreateBatchBar,
  renderDirEntries as renderDirBrowserEntries,
  selectedLaunchTarget as dirBrowserSelectedLaunchTarget,
  selectedLaunchTargetSummary as dirBrowserSelectedLaunchTargetSummary,
  visibleDirBatchPlan,
  visibleSelectableDirPaths as dirBrowserVisibleSelectableDirPaths,
} from "./dir_browser.js";
import {
  normalizeCreateSessionResponse,
  normalizeCreateSessionsBatchResponse,
  normalizeDirListResponse,
} from "./contracts.js";
import { responseJson as defaultResponseJson } from "./api_client.js";

export function shouldRetryDirListingFromBase(error, targetPath, groupName, options = {}) {
  if (options.retriedFromBase || !targetPath || groupName) {
    return false;
  }
  if (error?.status !== 403) {
    return false;
  }
  return String(error?.message || "").toLowerCase().includes("outside the allowed base directory");
}

export function batchFailureLines(results) {
  return results
    .filter((result) => !result?.ok)
    .map((result) => {
      const cwd = String(result?.cwd || "(unknown)");
      const message = result?.error?.message || result?.error?.code || "unknown error";
      return `${cwd} (${message})`;
    });
}

export function launchReceiptStatusLine(receipt) {
  if (!receipt) return "";
  const target = receipt.target_label || receipt.target_id || "target";
  if (receipt.outcome === "handoff") {
    const command = receipt.attach_hint || receipt.bootstrap_hint || "configure a safe ssh alias";
    return `Handoff ${target}: ${command}`;
  }
  if (receipt.outcome === "created") {
    const session = receipt.session_id || "session";
    if (receipt.local_cwd && receipt.remote_cwd) {
      return `Created ${session} on ${target}: ${receipt.local_cwd} -> ${receipt.remote_cwd}`;
    }
    if (receipt.local_override) {
      return `Created ${session} on local machine with explicit override`;
    }
    return `Created ${session} on ${target}`;
  }
  if (receipt.outcome === "blocked") {
    return receipt.message || `Launch blocked for ${target}`;
  }
  return receipt.message || `Launch ${receipt.outcome || "result"} for ${target}`;
}

export function createDirBrowserController(runtime) {
  const {
    state,
    el,
    apiFetch,
    responseJson = defaultResponseJson,
    setDirStatus,
    syncSheetActionAvailability,
    currentSession = () => null,
    closeSheets = () => {},
    refreshSessions = async () => {},
    selectSession = async () => {},
    setUtilityStatus = () => {},
    openSheet = () => {},
    focusActiveSheet = () => {},
    parentDir = () => "",
    storage = globalThis.localStorage,
    location = globalThis.window?.location,
    ElementClass = globalThis.Element,
    pathStorageKey = "swimmers.web.dirs.path",
    managedOnlyStorageKey = "swimmers.web.dirs.managed",
    renderDirBrowserView = null,
  } = runtime;

  function eventElement(event, target = event?.target) {
    return ElementClass && target instanceof ElementClass ? target : null;
  }

  function ensureDirBrowserBatchSelection() {
    return ensureDirBrowserBatchSelectionState(state.dirBrowser);
  }

  function visibleSelectableDirPaths() {
    return dirBrowserVisibleSelectableDirPaths(state.dirBrowser);
  }

  function selectedLaunchTarget() {
    return dirBrowserSelectedLaunchTarget(el, state.dirBrowser);
  }

  function selectedLaunchTargetSummary() {
    return dirBrowserSelectedLaunchTargetSummary(el, state.dirBrowser);
  }

  function launchTargetPayload() {
    return dirBrowserLaunchTargetPayload(el, state.dirBrowser);
  }

  function remoteInventoryTargetPayload() {
    const target = selectedLaunchTargetSummary();
    return String(target?.kind || "").trim().toLowerCase() === "swimmers_api"
      ? launchTargetPayload()
      : null;
  }

  function inventorySourceMatchesTarget(source, target) {
    const kind = String(source?.kind || "").trim().toLowerCase();
    if (target === null) {
      return kind === "local";
    }
    return kind === "remote_swimmers_api"
      && String(source?.target_id || "").trim() === target;
  }

  function remoteDirectoryWritesReadOnly() {
    return launchTargetPayload() !== null;
  }

  function launchTargetPreflightUnavailable() {
    const target = launchTargetPayload();
    return target !== null && !dirBrowserLaunchTargetById(state.dirBrowser, target);
  }

  function launchTargetPreviewForPath(path) {
    return dirBrowserLaunchTargetPreviewForPath(path, selectedLaunchTargetSummary());
  }

  function launchTargetStatusForPreview(preview) {
    return dirBrowserLaunchTargetStatusTextForPreview(preview);
  }

  function refreshCreateLaunchTargetBlocker() {
    const path = String(el.createCwd?.value || state.dirBrowser.path || "").trim();
    if (!path) {
      state.dirBrowser.singleLaunchBlocker = null;
      return null;
    }
    if (launchTargetPreflightUnavailable()) {
      state.dirBrowser.singleLaunchBlocker = null;
      return null;
    }
    const preview = launchTargetPreviewForPath(path);
    state.dirBrowser.singleLaunchBlocker = preview.blocked ? preview : null;
    return preview;
  }

  function syncCreateLaunchTargetStatus() {
    const preview = refreshCreateLaunchTargetBlocker();
    if (!preview) {
      return;
    }
    setDirStatus(launchTargetStatusForPreview(preview), preview.blocked);
  }

  function batchLaunchBlockersForPaths(paths) {
    if (launchTargetPreflightUnavailable()) {
      state.dirBrowser.batchLaunchBlockers = [];
      return [];
    }
    const blockers = dirBrowserLaunchTargetBlockersForPaths(paths, selectedLaunchTargetSummary());
    state.dirBrowser.batchLaunchBlockers = blockers;
    return blockers;
  }

  function renderCreateBatchBar() {
    renderDirBrowserCreateBatchBar({ el, dirBrowser: state.dirBrowser });
  }

  function currentDirListingPayload() {
    return {
      path: state.dirBrowser.path,
      entries: state.dirBrowser.entries,
      groups: state.dirBrowser.groups,
      overlay_label: state.dirBrowser.overlayLabel || undefined,
      launch_targets: state.dirBrowser.launchTargets,
      default_launch_target: state.dirBrowser.launchTarget,
      inventory_source: state.dirBrowser.inventorySource,
    };
  }

  function clearCreateBatchSelection() {
    clearDirBrowserBatchSelection({
      el,
      dirBrowser: state.dirBrowser,
      syncSheetActionAvailability,
    });
  }

  function handleCreateBatchVisibleAction() {
    const plan = visibleDirBatchPlan(visibleSelectableDirPaths(), state.dirBrowser.path, el.dirsPath.value);
    const selected = ensureDirBrowserBatchSelection();
    selected.clear();
    for (const path of plan.paths) selected.add(path);
    if (plan.firstPath) el.createCwd.value = plan.firstPath;
    batchLaunchBlockersForPaths(plan.paths);
    renderDirEntries(currentDirListingPayload());
    setDirStatus(plan.statusLabel, plan.statusMuted);
  }

  function handleDirCheckboxChange(event) {
    const target = eventElement(event);
    const plan = dirCheckboxChangePlan(event.type, target);
    if (plan.type === "ignore") return false;
    if (plan.type === "reset_checkbox") {
      plan.checkbox.checked = false;
      return true;
    }
    const selected = ensureDirBrowserBatchSelection();
    (plan.type === "add" ? selected.add : selected.delete).call(selected, plan.path);
    if (plan.type === "add") el.createCwd.value = plan.path;
    batchLaunchBlockersForPaths(Array.from(selected));
    renderCreateBatchBar();
    syncCreateLaunchTargetStatus();
    syncSheetActionAvailability();
    return true;
  }

  async function handleDirGroupChipClick(event, target = eventElement(event)) {
    const plan = dirGroupChipClickPlan(event.type, target, el.dirsManagedOnly.checked, state.dirBrowser.path, el.dirsPath.value);
    if (plan.type !== "filter") return false;
    state.dirBrowser.group = plan.group;
    state.dirBrowser.managedOnly = plan.managedOnly;
    el.dirsManagedOnly.checked = plan.managedOnly;
    storage.setItem(managedOnlyStorageKey, String(plan.managedOnly));
    clearCreateBatchSelection();
    await loadDirListing(plan.path, plan.managedOnly, plan.group);
    return true;
  }

  function renderDirEntries(response, options = {}) {
    renderDirBrowserEntries(response, {
      el,
      dirBrowser: state.dirBrowser,
      readOnly: state.readOnly,
      groupActionsReadOnly: state.readOnly || remoteDirectoryWritesReadOnly(),
      storage,
      pathStorageKey,
      managedOnlyStorageKey,
      setDirStatus,
      syncSheetActionAvailability,
      renderDirBrowserView,
      preferredLaunchTarget: options.preferredLaunchTarget,
    });
  }

  async function loadDirListing(
    path = el.dirsPath.value,
    managedOnly = el.dirsManagedOnly.checked,
    group = state.dirBrowser.group,
    options = {},
  ) {
    const targetPath = String(path || "").trim();
    const managed = Boolean(managedOnly);
    const groupName = String(group || "").trim();

    // Ordering guard: two overlapping navigations (slow dir A, then fast dir B)
    // race on apiFetch. Capture a monotonic seq so a late-resolving older request
    // cannot overwrite a newer request's rendered view or persisted path.
    const listingSeq = (state.dirBrowser.listingSeq || 0) + 1;
    state.dirBrowser.listingSeq = listingSeq;

    state.dirBrowser.loading = true;
    state.dirBrowser.managedOnly = managed;
    state.dirBrowser.group = groupName;
    el.dirsManagedOnly.checked = managed;
    storage.setItem(managedOnlyStorageKey, String(managed));
    setDirStatus("Loading directories...");
    try {
      const url = new URL("/v1/dirs", location.origin);
      if (targetPath) {
        url.searchParams.set("path", targetPath);
      }
      url.searchParams.set("managed_only", String(managed));
      if (groupName) {
        url.searchParams.set("group", groupName);
      }
      const target = remoteInventoryTargetPayload();
      if (target) {
        url.searchParams.set("target", target);
      }
      const response = await apiFetch(url.pathname + url.search);
      const payload = await responseJson(response, normalizeDirListResponse);
      if (!inventorySourceMatchesTarget(payload.inventory_source, target)) {
        const received = payload.inventory_source?.target_id || payload.inventory_source?.kind || "unknown";
        throw new Error(`directory inventory source mismatch: requested ${target || "local"}, received ${received}`);
      }
      // A newer navigation started while this request was in flight: drop this
      // stale payload before it renders or mutates path/persisted state.
      if (state.dirBrowser.listingSeq !== listingSeq) {
        return true;
      }
      renderDirEntries(payload, {
        preferredLaunchTarget: options.preferredLaunchTarget,
      });
      return true;
    } catch (error) {
      if (shouldRetryDirListingFromBase(error, targetPath, groupName, options)) {
        storage.removeItem(pathStorageKey);
        state.dirBrowser.path = "";
        state.dirBrowser.group = "";
        el.dirsPath.value = "";
        el.createCwd.value = "";
        setDirStatus("Saved directory was outside the repository root. Loading the default directory...");
        return loadDirListing("", managed, "", {
          retriedFromBase: true,
          preferredLaunchTarget: options.preferredLaunchTarget,
        });
      }
      setDirStatus(`Failed to load directories: ${error.message}`, true);
      return false;
    } finally {
      state.dirBrowser.loading = false;
      syncSheetActionAvailability();
    }
  }

  async function updateDirEntryGroupMembership(path, action, groupName, removeGroups = []) {
    const targetPath = String(path || "").trim();
    const targetGroup = String(groupName || "").trim();
    const sourceGroups = normalizedDirGroupList(removeGroups);
    if (!targetPath || !targetGroup || state.readOnly) {
      return;
    }
    if (remoteDirectoryWritesReadOnly()) {
      setDirStatus("Remote directory group edits are read-only from this server.", true);
      return;
    }

    const add = [];
    const remove = [];
    if (action === "remove") {
      remove.push(targetGroup);
    } else {
      add.push(targetGroup);
      if (action === "move") {
        remove.push(...sourceGroups.filter((group) => group !== targetGroup));
      }
    }

    setDirStatus("Updating directory group...");
    try {
      await apiFetch("/v1/dirs/group-memberships", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ path: targetPath, target: launchTargetPayload(), add, remove }),
      });
      await loadDirListing(
        state.dirBrowser.path || el.dirsPath.value,
        state.dirBrowser.managedOnly,
        state.dirBrowser.group,
      );
    } catch (error) {
      setDirStatus(`Failed to update group: ${error.message}`, true);
    }
  }

  async function warmDirBrowserOnStartup() {
    if (state.dirBrowser.loading || state.dirBrowser.entries.length > 0) {
      return;
    }
    await loadDirListing(state.dirBrowser.path || "", state.dirBrowser.managedOnly, state.dirBrowser.group);
  }

  function openCreateSheetForCwd(cwd, options = {}) {
    const path = String(cwd || "").trim();
    const launchTarget = String(options.launchTarget || "local").trim() || "local";
    const previousPath = String(state.dirBrowser.path || "");
    const previousTarget = String(state.dirBrowser.launchTarget || "local") || "local";
    const inventoryChanged = Boolean(path && (path !== previousPath || launchTarget !== previousTarget));
    if (path) {
      el.createCwd.value = path;
      el.dirsPath.value = path;
      state.dirBrowser.path = path;
    }
    if (inventoryChanged) {
      state.dirBrowser.entries = [];
      state.dirBrowser.groups = [];
      state.dirBrowser.overlayLabel = "";
    }
    state.dirBrowser.launchTarget = launchTarget;
    if (el.createLaunchTarget) {
      el.createLaunchTarget.value = launchTarget;
    }
    state.dirBrowser.singleLaunchBlocker = null;
    state.dirBrowser.batchLaunchBlockers = [];
    state.dirBrowser.group = "";
    clearCreateBatchSelection();
    syncSheetActionAvailability();
    openSheet("create");
  }

  function selectedBatchDirs() {
    return Array.from(ensureDirBrowserBatchSelection())
      .map((dir) => String(dir || "").trim())
      .filter(Boolean);
  }

  async function createBatchSessionsFromSheet(dirs, spawnTool, initialRequest) {
    const blockers = batchLaunchBlockersForPaths(dirs);
    if (blockers.length > 0) {
      const preview = blockers.slice(0, 3).map((blocker) => blocker.localCwd).join(", ");
      const overflow = blockers.length > 3 ? ` (+${blockers.length - 3} more)` : "";
      setDirStatus(`Remote batch has unmapped directories: ${preview}${overflow}`, true);
      return;
    }

    const response = await apiFetch("/v1/sessions/batch", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        dirs,
        spawn_tool: spawnTool || "grok",
        launch_target: launchTargetPayload(),
        initial_request: initialRequest || "",
      }),
    });
    const payload = normalizeCreateSessionsBatchResponse(await response.json());
    const results = Array.isArray(payload?.results) ? payload.results : [];
    const total = dirs.length;
    const successResults = results.filter((result) => result?.ok);
    const successCount = successResults.length;
    const failures = batchFailureLines(results);
    const failCount = failures.length;

    if (successCount > 0) {
      closeSheets();
      clearCreateBatchSelection();
      await refreshSessions();
      const firstSessionId = successResults.find((result) => result?.session?.session_id)?.session?.session_id;
      if (firstSessionId) {
        await selectSession(firstSessionId);
      }
    }

    if (failCount > 0) {
      const preview = failures.slice(0, 3).join("; ");
      const overflow = failCount > 3 ? ` (+${failCount - 3} more)` : "";
      const prefix = response.status === 207 ? "Batch send partial" : "Batch send failed";
      setUtilityStatus(`${prefix}: ${successCount}/${total} created. Failed: ${preview}${overflow}`, true, 6200);
      if (successCount === 0) {
        setDirStatus(`Batch send failed for all ${total}: ${preview}${overflow}`, true);
      }
      return;
    }

    const handoffResults = successResults.filter((result) => (
      result?.launch_receipt?.outcome === "handoff" && !result?.session?.session_id
    ));
    if (handoffResults.length > 0) {
      const first = launchReceiptStatusLine(handoffResults[0].launch_receipt);
      const overflow = handoffResults.length > 1 ? ` (+${handoffResults.length - 1} more)` : "";
      setDirStatus(`${first}${overflow}`, false);
      setUtilityStatus(`${handoffResults.length}/${total} handoff receipts ready`, false, 6200);
      return;
    }

    setUtilityStatus(`Batch send created ${successCount}/${total} sessions.`, false, 3600);
  }

  async function createSessionFromSheet() {
    // Re-entrancy guard: a second submit (double-click, or Enter then click)
    // while a create/batch request is in flight must not fire a duplicate POST.
    // This covers the batch path too, since it is only reached by delegation
    // from here.
    if (state.dirBrowser.creating) {
      return;
    }
    state.dirBrowser.creating = true;
    try {
      await createSessionFromSheetUnguarded();
    } catch (error) {
      // apiFetch throws on a hard (non-207) failure; surface it instead of
      // leaving an unhandled rejection with no user feedback. Covers the
      // form-submit and Enter paths, which (unlike spawn-here) had no catch.
      setDirStatus(`Failed to create session: ${error?.message || "create failed"}`, true);
      syncSheetActionAvailability();
    } finally {
      state.dirBrowser.creating = false;
    }
  }

  async function createSessionFromSheetUnguarded() {
    if (state.readOnly) {
      return;
    }

    const batchDirs = selectedBatchDirs();
    const cwd = el.createCwd.value.trim();
    const initialRequest = el.createRequest.value.trim();
    const spawnTool = el.createTool.value;

    if (batchDirs.length > 0) {
      await createBatchSessionsFromSheet(batchDirs, spawnTool, initialRequest);
      return;
    }

    if (launchTargetPreflightUnavailable()) {
      state.dirBrowser.singleLaunchBlocker = null;
    } else {
      const launchPreview = launchTargetPreviewForPath(cwd);
      state.dirBrowser.singleLaunchBlocker = launchPreview.blocked ? launchPreview : null;
      if (launchPreview.blocked) {
        setDirStatus(launchTargetStatusForPreview(launchPreview), true);
        return;
      }
    }

    const response = await apiFetch("/v1/sessions", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        cwd: cwd || null,
        spawn_tool: spawnTool,
        launch_target: launchTargetPayload(),
        initial_request: initialRequest || null,
      }),
    });

    const payload = normalizeCreateSessionResponse(await response.json());
    const created = payload?.session;
    if (created?.session_id) {
      closeSheets();
      await refreshSessions();
      await selectSession(created.session_id);
    }
    const receiptLine = launchReceiptStatusLine(payload?.launch_receipt);
    if (receiptLine) {
      setDirStatus(receiptLine, false);
    }
  }

  async function openCreateSheet() {
    const selected = currentSession();
    const preferredPath = String(el.createCwd.value || state.dirBrowser.path || selected?.cwd || "").trim();
    const initialPath = preferredPath || state.dirBrowser.path || "";
    ensureDirBrowserBatchSelection().clear();
    state.dirBrowser.group = "";
    if (initialPath) {
      el.createCwd.value = initialPath;
      el.dirsPath.value = initialPath;
    }
    if (typeof state.dirBrowser.managedOnly !== "boolean") {
      state.dirBrowser.managedOnly = false;
    }
    el.dirsManagedOnly.checked = state.dirBrowser.managedOnly;
    renderCreateBatchBar();
    if (!state.dirBrowser.entries.length || (initialPath && initialPath !== state.dirBrowser.path)) {
      await loadDirListing(initialPath, state.dirBrowser.managedOnly, "");
    } else {
      renderDirEntries(currentDirListingPayload());
    }
    focusActiveSheet();
  }

  async function handleCreateFormSubmit(event) {
    event.preventDefault();
    await createSessionFromSheet();
  }

  async function handleCreateLaunchTargetChange() {
    const previousTarget = state.dirBrowser.launchTarget || "local";
    const nextTarget = selectedLaunchTarget();
    state.dirBrowser.launchTarget = nextTarget;
    renderCreateBatchBar();
    batchLaunchBlockersForPaths(selectedBatchDirs());
    syncCreateLaunchTargetStatus();
    syncSheetActionAvailability();
    if (nextTarget !== previousTarget && (state.dirBrowser.path || el.dirsPath.value || state.dirBrowser.entries.length)) {
      const loaded = await loadDirListing(
        state.dirBrowser.path || el.dirsPath.value,
        state.dirBrowser.managedOnly,
        state.dirBrowser.group,
        { preferredLaunchTarget: nextTarget },
      );
      if (!loaded) {
        state.dirBrowser.launchTarget = previousTarget;
        if (el.createLaunchTarget) {
          el.createLaunchTarget.value = previousTarget;
        }
        renderCreateBatchBar();
        batchLaunchBlockersForPaths(selectedBatchDirs());
        refreshCreateLaunchTargetBlocker();
        syncSheetActionAvailability();
      }
    }
  }

  function handleDirsSearchInput() {
    state.dirBrowser.search = String(el.dirsSearch.value || "");
    renderDirEntries(currentDirListingPayload());
  }

  function handleCreateBatchClearClick() {
    clearCreateBatchSelection();
    setDirStatus("Batch selection cleared.");
  }

  function handleCreateCwdInput() {
    el.dirsPath.value = el.createCwd.value;
    syncCreateLaunchTargetStatus();
    syncSheetActionAvailability();
  }

  function handleDirsManagedOnlyChange() {
    state.dirBrowser.managedOnly = Boolean(el.dirsManagedOnly.checked);
    storage.setItem(managedOnlyStorageKey, String(state.dirBrowser.managedOnly));
    syncSheetActionAvailability();
    void loadDirListing(el.dirsPath.value, state.dirBrowser.managedOnly);
  }

  function handleDirsPathInput() {
    syncSheetActionAvailability();
  }

  function handleDirsPathKeydown(event) {
    if (event.key === "Enter") {
      event.preventDefault();
      state.dirBrowser.group = "";
      clearCreateBatchSelection();
      void loadDirListing(el.dirsPath.value, el.dirsManagedOnly.checked, "");
    }
  }

  async function handleDirsLoadButtonClick() {
    state.dirBrowser.group = "";
    clearCreateBatchSelection();
    await loadDirListing(el.dirsPath.value, el.dirsManagedOnly.checked, "");
  }

  async function handleDirsSpawnHereClick() {
    if (state.readOnly) {
      return;
    }
    const path = String(state.dirBrowser.path || el.dirsPath.value || el.createCwd.value || "").trim();
    if (!path) {
      return;
    }
    clearCreateBatchSelection();
    el.createCwd.value = path;
    el.dirsPath.value = path;
    try {
      await createSessionFromSheet();
    } catch (error) {
      setDirStatus(`Failed to spawn here: ${error.message}`, true);
      syncSheetActionAvailability();
    }
  }

  async function handleDirsUpButtonClick() {
    const parent = parentDir(el.dirsPath.value);
    if (parent) {
      state.dirBrowser.group = "";
      clearCreateBatchSelection();
      el.dirsPath.value = parent;
      el.createCwd.value = parent;
      await loadDirListing(parent, el.dirsManagedOnly.checked, "");
    }
  }

  async function handleDirsListClick(event) {
    const target = eventElement(event);
    if (!target) {
      return;
    }
    if (target.closest(".dir-open-url")) {
      return;
    }

    if (await handleDirGroupChipClick(event, target)) {
      return;
    }

    const groupActionPlan = dirGroupMembershipClickPlan(event.type, target);
    if (groupActionPlan.type === "membership") {
      await updateDirEntryGroupMembership(groupActionPlan.path, groupActionPlan.action, groupActionPlan.group, groupActionPlan.removeGroups);
      return;
    }

    const rowPlan = dirRowClickPlan(event.type, target);
    if (rowPlan.type !== "row") {
      return;
    }
    const path = rowPlan.path;
    el.dirsPath.value = path;
    el.createCwd.value = path;
    if (rowPlan.hasChildren) {
      state.dirBrowser.group = "";
      clearCreateBatchSelection();
      await loadDirListing(path, el.dirsManagedOnly.checked, "");
      return;
    }
    setDirStatus(`Selected ${path}`);
    syncSheetActionAvailability();
  }

  return {
    batchFailureLines,
    clearCreateBatchSelection,
    createBatchSessionsFromSheet,
    createSessionFromSheet,
    currentDirListingPayload,
    ensureDirBrowserBatchSelection,
    handleCreateBatchClearClick,
    handleCreateBatchVisibleAction,
    handleCreateCwdInput,
    handleCreateFormSubmit,
    handleCreateLaunchTargetChange,
    handleDirCheckboxChange,
    handleDirGroupChipClick,
    handleDirsListClick,
    handleDirsLoadButtonClick,
    handleDirsManagedOnlyChange,
    handleDirsPathInput,
    handleDirsPathKeydown,
    handleDirsSearchInput,
    handleDirsSpawnHereClick,
    handleDirsUpButtonClick,
    launchTargetPayload,
    loadDirListing,
    openCreateSheet,
    openCreateSheetForCwd,
    renderCreateBatchBar,
    renderDirEntries,
    selectedBatchDirs,
    selectedLaunchTarget,
    shouldRetryDirListingFromBase,
    updateDirEntryGroupMembership,
    visibleSelectableDirPaths,
    warmDirBrowserOnStartup,
  };
}
