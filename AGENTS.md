# Rastera project instructions

- Read README.md, docs/architecture.md, and docs/development-status.md before changing behavior. The blueprint is long-term direction, not a feature inventory.
- Keep geometry and reusable GPU rendering in rastera-core; desktop controls and native viewport management belong in rastera-desktop. Keep source pixel dimensions independent of window size. Source bounds are half-open, top-left origin.
- All displays consume the same offscreen canvas. Never bake operator controls or selection overlays into clean outputs.
- Keep this milestone small. Do not add mock success states for NDI, DeckLink, capture, decoding, audio, or slides.
- Run cargo fmt --all -- --check, cargo check --workspace --locked, cargo test --workspace --locked, and cargo clippy --workspace --all-targets --locked -- -D warnings. Include Cargo.lock in review/commits.
- Report automated checks separately from native launch and human visual/hardware verification. Do not claim frame synchronization, sustained performance, platform support, or integrations without evidence.
- Preserve existing work. Do not commit or push without the user's instruction.
