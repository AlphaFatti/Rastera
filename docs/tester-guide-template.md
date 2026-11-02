RASTERA {{VERSION}} — EARLY DEVELOPMENT PREVIEW

Apple silicon (arm64) Macs only; not Universal, no Intel or Windows support.
Minimum macOS 14.0. Tested on Apple M1 Pro / macOS 15.5 only; other versions,
chips, devices and another-Mac testing remain pending. This is not a production
show release. No tester account or telemetry setup is required.

Source commit: {{SOURCE_COMMIT}}
Signing: {{SIGNING}}

INSTALL AND OPEN

1. Keep the DMG, this guide and SHA256SUMS together. In Terminal, run
   `shasum -a 256 -c SHA256SUMS` in their folder to check the delivered files.
   If the build-info JSON is listed, keep that alongside them too.
2. Open Rastera-{{VERSION}}-macOS-arm64.dmg. Drag Rastera.app to Applications.
   Eject the disk image, then open the installed app from Finder or Launchpad.
   No Rust, Cargo, Python, Homebrew, ffmpeg, checkout or developer variables
   are needed to run this application.
3. {{OPENING}}

FIRST TEST (use noncritical media at low speaker volume)

- Fresh profiles have audio/video outputs disabled and no capture input enabled.
  If you previously used Rastera, your existing recovery/preferences remain;
  use Disable All Outputs first. Audio activation always starts off.
- Import a PNG/JPEG, add text, or Insert Video / GIF. Try a quick scene and
  source crop/corner pin. Live Edit changes clean outputs immediately.
- Add a video's embedded audio tracks, or import a WAV/AIFF. In Audio Mixer,
  explicitly select track/channel mappings. Two-channel defaults to Stereo;
  choose Dual Mono for separate channels. Four-channel defaults to Dual Stereo;
  choose Quad Mono for four strips. Alternate-language tracks are not combined
  automatically. Each strip has DSP, mute/fader and ten stereo bus sends.
- Pick an available output device and channel pair for a bus; enable that send,
  then choose Enable Audio Outputs. Start quiet. Check correct speakers and
  channel isolation. Duplicate visual layers must not double the audio.
- Capture and microphone access require explicit Add/select/enable actions and
  macOS authorization. Allow Camera/Microphone only when you intend to use them.
  A camera picture does not automatically enable microphone audio. Missing or
  denied sources report status rather than substituting another device.
- In Viewports, enable a Windowed output, then choose Fullscreen and its target
  display. One fullscreen viewport per display. Inspect aspect-fit, correct
  physical display and clean picture (no editor handles). Never assign your
  operator display until you know the recovery shortcut below.
- Save a project, quit and reopen it; verify media references and mappings.
  Projects keep logical audio settings; physical device patches are local.
  Enable outputs deliberately. Test play/pause/seek/loop and report gaps.

RECOVERY CONTROLS

Enable Audio Outputs: master audio activation; always off at startup.
Mute All Audio / Shift-Command-M: app-scoped audio emergency mute.
Disable All Outputs / Shift-Command-Escape: app-scoped clean VIDEO recovery;
this does not mute audio. Command-Tab back to Rastera first if another app is
active. Command-Q quits normally and flushes recovery.
Restore Last Session is separate from Restore enabled outputs on startup.
Video output recall is opt-in; do not enable it unintentionally.

KNOWN LIMITATIONS — PLEASE DO NOT USE FOR A CRITICAL SHOW

Source seek/loop PCM gaps are reproduced; short loops are not seamless.
Operator UI/video/capture-conditioned transition pauses remain documented.
Counts/timestamps are API evidence, not physical scanout verification. A user
reported click-free playback after substituting their own VOG file; that does
not establish a fixture defect or general audio continuity. Physical A/V timing,
loopback calibration, broad multichannel capture/hardware, disconnect/reconnect,
and endurance beyond the recorded ten-minute trial remain pending. This release
does not resolve those issues. UI redesign, slides and cue/queue are future work.
The build Mac already had Gatekeeper assessments disabled; this was not changed.
Its quarantined-copy launch does not verify opening on an enforcing Mac.

YOUR DATA AND UNINSTALL

Default recovery/preferences/device patches live under
~/Library/Application Support/Rastera/ (including session/). Projects and media
remain wherever you save them. Media references are not embedded copies; keep
relative media paths with your project when sharing. Back up projects yourself.
To uninstall, quit Rastera and remove Rastera.app from Applications. This does
not delete projects, media or the Application Support folder. Do not delete that
folder unless you deliberately want to discard recovery/preferences/patches.
This package contains no personal projects, recordings, profiles or credentials.

FEEDBACK

Send Mac model/chip, macOS version, connected displays/audio/capture devices,
media container/codec/sample rate/channel-track layout, project/output settings,
steps to reproduce, expected versus actual result and whether it happens at
seek/loop or minimize/restore. Include exact permission/Gatekeeper messages.
Share only media/logs/projects you intend to disclose; no automatic upload runs.
