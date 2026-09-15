# T-901 — Plugin GUIs as floating windows owned by the sandbox

- **Tier:** Opus (platform windowing, FFI, sandbox; blocking findings only)
- **Depends on:** M8 (CLAP T-803, VST3 T-806, LV2 T-807, JSFX T-808, sandbox T-801/T-802, H-36 transport, T-810 state).
- **Read first:**
  - CLAUDE.md, MEMORY.md (the T-801–T-808, T-805, T-810, H-36 and H-40 entries; notes left for T-901: VST3 `createView` unused, `mark_dirty`/GUI state not refreshed at save, LV2 GUIs and atom messages not done, JSFX built without `@gfx`);
  - docs/adr/ADR-008 §7 ("Editor windows (M9) are owned by the sandbox": plugin GUIs run on the sandbox's main thread as floating top-level windows) and all amendments;
  - ADR-005, ADR-007.
- **Platforms:** Linux first (X11, and XWayland under Wayland: the owner runs Hyprland), Windows (HWND) and macOS (NSView) behind cfg, compiling in `just check-cross`.

## Goal
Clicking "Open plugin window" on a plugin slot opens the plugin's own editor as a floating window run by the sandbox process. Moving a knob there updates the rack's parameter view and automation. If the plugin crashes, the GUI window dies with the sandbox and PowerVoice stays up.

## Scope (in)
1. **Sandbox windowing:**
   - an event loop on the sandbox main thread that coexists with the 10 ms idle tick;
   - create a top-level window (X11 via a minimal Xlib/xcb binding, loaded at runtime or with the smallest dependency; justify it in an ADR-008 amendment), sized by the plugin, resizable if the plugin allows;
   - title "‹Plugin› — PowerVoice";
   - kept above the PowerVoice window where the platform allows (transient-for hint), and closed with its slot, its document or the app.
2. **Format hosts:**
   - **CLAP** `gui` extension: floating or embedded into our window;
   - **VST3** `IPlugView` (`createView`, `attached`, resize through `IPlugFrame`), plus `performEdit` round trips;
   - **LV2** UI through suil if lilv/suil is available at runtime (graceful "no GUI" otherwise; `ui:X11UI` only);
   - **JSFX:** skip `@gfx` in v1 (document it), unless building LICE/SWELL is trivial, which it isn't.
3. **Sync:** GUI parameter edits reach the host (CLAP `params.flush` and output events, VST3 `performEdit`) and the rack's generic parameter UI and undo/dirty state. Host automation changes are reflected in the GUI. At save time, state is captured including GUI-only state (`mark_dirty`, VST3 `getState`), which fixes the T-810 caveat.
4. **UI:** an "Open plugin window" button (and double-click) on sandboxed slots with a GUI, the window's open state shown on the slot, and "Close all plugin windows" on document close. With no GUI support the button is disabled with a tooltip. i18n; the T-702 lint applies.
5. **Robustness:**
   - a crashing GUI takes only its sandbox down, following the existing restart-once policy, and the window disappears;
   - a hanging GUI trips the watchdog;
   - no window leaks on slot removal or app quit.

## Tests
- A windowing smoke test on X11/XWayland if a display is available: create and destroy a window with a GUI test plugin. Extend the CLAP test plugin with a trivial GUI (an X11 window that sends a param change on a timer), and skip without `DISPLAY`/`WAYLAND_DISPLAY`.
- The param round trip GUI → host → rack.
- State captured at save after a GUI-only change.
- A crash in the GUI → only its sandbox dies.
- No window after slot removal.
- `check-cross` compiles the Windows HWND stub.

Don't open windows on the owner's workspace from tests without a display check. Prefer a nested or virtual X server if one is available without installing packages (`Xvfb`/`Xephyr`, if present); otherwise skip.

`just check` and `just check-cross` must pass.
