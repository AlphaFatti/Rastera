# Media and scenes development

Branch `feature/media-scenes`, based on capture checkpoint `c813e6a`. Version remains 0.0.2-dev; v0.0.1 is unchanged. Work is delivered sequentially; later sections record only implemented features and actual checks.

## Stage 1: source cropping

Schema 6 explicitly migrates versions 1–5, preserving IDs, geometry and full-source crop defaults. Image and camera layer instances store normalized left/top/right/bottom bounds. Duplicated cameras keep one definition/session and independent crops. The layer rectangle is the visible cropped rectangle; UVs select its source region. Cropping changes that rectangle while preserving underlying pixel scale and full-source position, rather than stretching surviving pixels. Reset restores the uncropped geometry; resize operates on the visible aspect. Normalized bounds retain their meaning when a camera mode changes.

Select an image/camera, edit **Source crop · pixels**, or enable **Edit crop with corner handles**. Turn that mode off to resize. Dragging the body still moves the object. Text does not offer source cropping. Crop is undoable; a completed handle gesture produces one history entry. Viewports mode and Option-click behavior are unchanged. Controls report requested camera dimensions, which equal validated delivered dimensions for a Live camera; unavailable cameras use their persisted mode.

Automated workspace tests include crop/reset pixel-scale geometry, empty/NaN rejection, crossing-handle constraints, independent shared-camera crops and undo. Native picture/handle verification for this stage is pending. Existing operator-transition, screen-capture and physical-display limitations remain. No production acceptance is claimed.
