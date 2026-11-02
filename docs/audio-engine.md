# Audio engine development

Branch feature/audio-engine, from f51e1b8. Existing tags and main remain unchanged. Planned order is audio, slides, playback queues, then UI overhaul; this branch does not implement slides/queues.

## Stage 1 — startup viewport recall (November 2, 2026)

The user reports that corner pinning appears to work, imported videos resume and the camera reconnects on launch. Those are scoped observations, not general continuity acceptance.

Two real defects were corrected without replacing recovery: final shutdown reused the previous periodic output set (a recent enable/disable could be missed); and unsaved sessions had no persistent destination namespace. Shutdown now captures logical activation and native geometry before consumer retirement. Unsaved destinations are scoped by machine + persistent project identity + an explicit unsaved-session discriminator, separately from saved project paths. Existing missing/ambiguous/conflicting destination resolution and Disable All Outputs remain.

The real installed profile originally remembered no enabled outputs. A private backup was made outside the repository. Restore Last Session and Restore enabled outputs are now enabled as requested; existing logical Left/Right were explicitly activated, Left windowed and Right assigned to the attached Crestron display. No scene/layer/viewport geometry was replaced.

On Apple M1 Pro/macOS 15.5, normal Command-Q (PID 9350) retained both enabled definitions and exact Right display UUID. Actual Launchpad search/tile click relaunched the installed executable without arguments (PID 9706, parent launchd). Native window enumeration asserted identical before/after bounds: Left x20 y542 w960 h568, Right x1800 y0 w1920 h1080 in Quartz screen coordinates; the operator bounds also matched. Document equality and two enabled outputs were asserted after restoration. Windowed geometry remains in AppKit points in recovery; fullscreen covers NSScreen's exact assigned frame (AppKit y89 here). These are native placement assertions, not physical picture/scanout synchronization checks.

Audio activation will remain separate from this video-output preference. Existing operator/capture-conditioned pause limitations and wider hardware acceptance remain pending.
