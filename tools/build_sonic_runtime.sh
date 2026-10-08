#!/bin/bash
set -euo pipefail
source ~/.cargo/env
src='/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android/touchHLE-src'
dst='/home/ben/touchHLE-a64-integration'
cp "$src/src/frameworks/core_animation.rs" "$dst/src/frameworks/core_animation.rs"
cp -r "$src/src/frameworks/core_animation/." "$dst/src/frameworks/core_animation/"
cp "$src/src/frameworks/core_graphics/cg_context.rs" "$dst/src/frameworks/core_graphics/cg_context.rs"
cp "$src/src/frameworks/core_graphics/cg_bitmap_context.rs" "$dst/src/frameworks/core_graphics/cg_bitmap_context.rs"
cp "$src/src/frameworks/uikit/ui_graphics.rs" "$dst/src/frameworks/uikit/ui_graphics.rs"
cp "$src/src/frameworks/uikit.rs" "$dst/src/frameworks/uikit.rs"
cp "$src/src/frameworks/uikit/ui_view.rs" "$dst/src/frameworks/uikit/ui_view.rs"
cp "$src/src/frameworks/uikit/ui_view/ui_table_view.rs" "$dst/src/frameworks/uikit/ui_view/ui_table_view.rs"
cp "$src/src/frameworks/uikit/ui_view/ui_table_view_cell.rs" "$dst/src/frameworks/uikit/ui_view/ui_table_view_cell.rs"
cp "$src/src/frameworks/uikit/ui_view/ui_window.rs" "$dst/src/frameworks/uikit/ui_view/ui_window.rs"
cp "$src/src/frameworks/foundation/ns_index_path.rs" "$dst/src/frameworks/foundation/ns_index_path.rs"
cp "$src/src/frameworks/audio_toolbox/audio_unit.rs" "$dst/src/frameworks/audio_toolbox/audio_unit.rs"
cp "$src/src/frameworks/foundation/ns_bundle.rs" "$dst/src/frameworks/foundation/ns_bundle.rs"
cp "$src/src/frameworks/uikit/ui_view/ui_web_view.rs" "$dst/src/frameworks/uikit/ui_view/ui_web_view.rs"
cp "$src/src/frameworks/uikit/ui_view/ui_control/ui_slider.rs" "$dst/src/frameworks/uikit/ui_view/ui_control/ui_slider.rs"
cp "$src/src/objc.rs" "$dst/src/objc.rs"
cp "$src/src/objc/messages.rs" "$dst/src/objc/messages.rs"
cp "$src/src/lib.rs" "$dst/src/lib.rs"
cp "$src/src/bundle.rs" "$dst/src/bundle.rs"
cp "$src/src/options.rs" "$dst/src/options.rs"
cp "$src/OPTIONS_HELP.txt" "$dst/OPTIONS_HELP.txt"
cp "$src/src/frameworks/uikit/ui_device.rs" "$dst/src/frameworks/uikit/ui_device.rs"
cd "$dst"
export CARGO_TARGET_DIR=/home/ben/touchHLE-a64/target
cargo test --lib
if [[ "${PLAYCOVER_NATIVE_TEST_ONLY:-0}" == 1 ]]; then exit 0; fi
bash "$src/../tools/build_repository_ui.sh"
