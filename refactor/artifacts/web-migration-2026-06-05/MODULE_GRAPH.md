# Web Module Graph

Run id: `web-migration-2026-06-05`
Artifact path: `refactor/artifacts/web-migration-2026-06-05/`

Command used:

```sh
node -e "const fs=require('fs'); const path=require('path'); const files=fs.readdirSync('src/web').filter(f=>f.endsWith('.js')).sort(); for (const f of files){ const s=fs.readFileSync(path.join('src/web',f),'utf8'); const deps=[...s.matchAll(/from\\s+['\"](\\.\\/[^'\"]+)['\"]|import\\s*\\(\\s*['\"](\\.\\/[^'\"]+)['\"]/g)].map(m=>(m[1]||m[2]).replace(/^\\.\\//,'')); console.log(f+' -> '+(deps.length ? deps.join(', ') : '(none)')); }"
```

Summary:

- JS modules: `48`
- Import declarations: `70`
- Unique dependency pairs: `68`
- Root module: `app.js`
- Direct imports from `app.js`: `32`

Graph:

```text
agent_context_refresh.js -> (none)
api_client.js -> (none)
app.js -> rendered_surface.js, input_support.js, app_event_handlers.js, terminal_stage_controller.js, terminal_focus.js, send_controller.js, terminal_input.js, terminal_zoom_input.js, thought_config_sheet.js, native_desktop_sheet.js, terminal_surface_setup.js, terminal_surface_controller.js, terminal_runtime.js, session_socket_controller.js, terminal_resize.js, session_refresh.js, terminal_workbench_controller.js, mermaid_artifact.js, mermaid_artifact_controller.js, terminal_safety.js, terminal_search_links.js, terminal_status.js, terminal_protocol.js, dir_browser_controller.js, command_palette_controller.js, trogdor_logic.js, trogdor_state.js, trogdor_surface_controller.js, workbench_render.js, surface_model.js, api_client.js, session_persistence.js
app_event_bindings.js -> (none)
app_event_handlers.js -> app_event_bindings.js, command_palette.js, global_shortcut_dispatch.js, input_support.js, trogdor_event_bindings.js
command_palette.js -> (none)
command_palette_controller.js -> command_palette.js
dir_browser.js -> terminal_safety.js
dir_browser_controller.js -> dir_browser.js
global_shortcut_dispatch.js -> (none)
input_support.js -> surface_action_plans.js
mermaid_artifact.js -> (none)
mermaid_artifact_controller.js -> mermaid_artifact.js
native_desktop_sheet.js -> (none)
rendered_surface.js -> rendered_surface_draw.js
rendered_surface_draw.js -> (none)
send_controller.js -> send_sheet.js
send_sheet.js -> (none)
session_persistence.js -> (none)
session_refresh.js -> (none)
session_socket_controller.js -> terminal_protocol.js
surface_action_plans.js -> (none)
surface_model.js -> trogdor_logic.js
terminal_focus.js -> input_support.js
terminal_input.js -> input_support.js, terminal_protocol.js
terminal_protocol.js -> (none)
terminal_resize.js -> input_support.js
terminal_runtime.js -> (none)
terminal_safety.js -> (none)
terminal_search_links.js -> (none)
terminal_stage_controller.js -> rendered_surface.js, input_support.js, terminal_protocol.js
terminal_status.js -> (none)
terminal_surface_controller.js -> (none)
terminal_surface_setup.js -> input_support.js
terminal_workbench_controller.js -> agent_context_refresh.js, workbench_dom.js, workbench_refresh.js, workbench_render.js
terminal_zoom_input.js -> input_support.js
thought_config_sheet.js -> (none)
trogdor_dom_logic.js -> (none)
trogdor_event_bindings.js -> trogdor_logic.js
trogdor_logic.js -> trogdor_dom_logic.js, trogdor_dom_logic.js
trogdor_render.js -> trogdor_logic.js
trogdor_state.js -> trogdor_logic.js
trogdor_surface_controller.js -> trogdor_logic.js, trogdor_render.js
workbench_dom.js -> (none)
workbench_log_lens.js -> workbench_records.js
workbench_records.js -> (none)
workbench_refresh.js -> workbench_render.js
workbench_render.js -> workbench_log_lens.js, workbench_log_lens.js
```

Notes:

- `trogdor_logic.js` imports from `trogdor_dom_logic.js` in two declarations.
- `workbench_render.js` imports from `workbench_log_lens.js` in two
  declarations.
- The Axum asset route graph is captured separately in
  `route-source-manifest.json`.
