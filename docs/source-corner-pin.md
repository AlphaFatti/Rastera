# Source corner pin and installed development app

November 2, 2026. Branch `feature/source-corner-pin` starts at d81ecad, preserving 0.0.2-dev and v0.0.1. The broader UI overhaul is deferred.

## Core geometry

Layer instances own a `Pin`, four normalized destination points ordered source TL, TR, BL, BR. Canvas coordinates have a top-left origin and positive Y down. The clockwise polygon order is TL, TR, BR, BL. Identity is (0,0), (1,0), (0,1), (1,1). `Crop::full_rect(layer.rect)` reconstructs the stable full-source plane. A double-precision homography maps full-source unit coordinates into normalized destination coordinates; the full rect maps these into canvas pixels. Crop bounds select a subdomain of this same mapping, so surviving source points stay fixed rather than stretching into the previous quad. Reset Pin changes only Pin, preserving crop, ordinary placement and source reference.

Rendering carries the homogeneous denominator into clip-space W and uses perspective UV interpolation. Both triangles therefore implement one projective transform, without an affine diagonal seam. Core geometry supplies forward/inverse transforms, visible corners, half-open polygon hit testing and anchored whole-shape resizing. Moving/resizing the ordinary rect transforms the full pinned plane; text resize still changes wrapping, while pin-only edits reuse the cached unchanged text tile. Normalized pin data never changes capture/playback definitions.

Finite, clockwise convex, invertible configurations are required. Crossed/concave/collapsed shapes, projective poles and poorly conditioned transforms are rejected. Conditioning thresholds: normalized cross products >1e-6 of squared span, positive homogeneous W ≥1e-4 with ≤10000 ratio, inverse norm product ≤1e8. Visible corners must lie within ±65536 canvas pixels; ordinary placement limits remain. Off-canvas valid geometry is clipped by the canvas render target.

Schema 10 explicitly migrates v1–v9 layers to identity pins, preserving project/layer/source/viewport IDs. Recovery journals use the same document migration, including their last-explicit-save document. Scene/layer duplication, history and recovery preserve independent pin instances. The four-texture producer, frame-boundary adoption, bounded generations, completion-held leases and GPU-only canvas display path are unchanged.

## Editor controls

Canvas mode has Transform / Crop / Corner Pin tools; Viewports selection and Option-click cycling are unchanged. Yellow TL/TR/BL/BR handles and numeric visible-corner canvas X/Y fields belong to Corner Pin. Source corner labels remain attached to source orientation even after rotating the shape. Reset Corner Pin restores rectangular mapping. The inspector marks active warps. Selection outlines and hit targets use the warped polygon; body dragging translates the whole source. Transform resizes the whole shape with the opposite visible corner anchored. Crop handles use the inverse full-source homography; text remains editable but has no source crop.

Handle hit radii derive from preview backing pixels/Retina scale and the actual fit rectangle. Each completed gesture has one history entry; selection alone has none. Escape cancels the gesture. Tool/mode changes finish it safely. Numeric/reset edits use normal history. Invalid edits keep the last valid geometry and show a warning. All handles/guides/labels remain exclusively in the operator overlay.

## Verification status

Pure geometry/migration/shared-source tests and native checks are reported separately below as they complete. No sustained performance, calibrated color, physical scanout synchronization or pause fix is claimed. Existing operator/capture-conditioned pauses and unverified hardware cases remain. Installation/signing/launcher results will be recorded separately from picture observations.
