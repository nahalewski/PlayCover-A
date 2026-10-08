#!/bin/bash
set -euo pipefail
source ~/.cargo/env
src='/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android/touchHLE-src'
dst='/home/ben/touchHLE-a64-integration'
copy_frozen_source() {
    if ! cmp -s -- "$1" "$2"; then
        cp -- "$1" "$2"
    fi
}
copy_frozen_source "$src/src/frameworks/core_graphics/cg_bitmap_context.rs" "$dst/src/frameworks/core_graphics/cg_bitmap_context.rs"
copy_frozen_source "$src/src/frameworks/core_graphics/cg_image.rs" "$dst/src/frameworks/core_graphics/cg_image.rs"
copy_frozen_source "$src/src/frameworks/core_graphics/cg_data_provider.rs" "$dst/src/frameworks/core_graphics/cg_data_provider.rs"
copy_frozen_source "$src/src/frameworks/audio_toolbox/audio_unit.rs" "$dst/src/frameworks/audio_toolbox/audio_unit.rs"
copy_frozen_source "$src/src/frameworks/media_player/movie_player.rs" "$dst/src/frameworks/media_player/movie_player.rs"
copy_frozen_source "$src/src/frameworks/core_animation/ca_layer.rs" "$dst/src/frameworks/core_animation/ca_layer.rs"
copy_frozen_source "$src/src/frameworks/core_animation/composition.rs" "$dst/src/frameworks/core_animation/composition.rs"
copy_frozen_source "$src/src/window.rs" "$dst/src/window.rs"
copy_frozen_source "$src/src/frameworks/opengles/eagl.rs" "$dst/src/frameworks/opengles/eagl.rs"
copy_frozen_source "$src/src/frameworks/uikit/ui_view/ui_control/ui_segmented_control.rs" "$dst/src/frameworks/uikit/ui_view/ui_control/ui_segmented_control.rs"
copy_frozen_source "$src/src/a64.rs" "$dst/src/a64.rs"
copy_frozen_source "$src/src/a64_legacy_cpp.rs" "$dst/src/a64_legacy_cpp.rs"
copy_frozen_source "$src/src/a64_legacy.rs" "$dst/src/a64_legacy.rs"
copy_frozen_source "$src/src/lib.rs" "$dst/src/lib.rs"
copy_frozen_source "$src/src/libc/mach/host.rs" "$dst/src/libc/mach/host.rs"
copy_frozen_source "$src/src/frameworks/foundation/ns_user_defaults.rs" "$dst/src/frameworks/foundation/ns_user_defaults.rs"
copy_frozen_source "$src/src/frameworks/foundation/ns_process_info.rs" "$dst/src/frameworks/foundation/ns_process_info.rs"
copy_frozen_source "$src/src/a64_exports.rs" "$dst/src/a64_exports.rs"
copy_frozen_source "$src/src/a64_dyld_slide.rs" "$dst/src/a64_dyld_slide.rs"
copy_frozen_source "$src/src/a64_dyld_slide_tests.rs" "$dst/src/a64_dyld_slide_tests.rs"
copy_frozen_source "$src/src/a64_dyld_helpers.rs" "$dst/src/a64_dyld_helpers.rs"
copy_frozen_source "$src/src/a64_tlv.rs" "$dst/src/a64_tlv.rs"
copy_frozen_source "$src/src/a64_tlv_bootstrap.rs" "$dst/src/a64_tlv_bootstrap.rs"
copy_frozen_source "$src/src/a64_tlv_storage.rs" "$dst/src/a64_tlv_storage.rs"
copy_frozen_source "$src/src/a64_tlv_storage_tests.rs" "$dst/src/a64_tlv_storage_tests.rs"
copy_frozen_source "$src/src/a64_tlv_tests.rs" "$dst/src/a64_tlv_tests.rs"
copy_frozen_source "$src/src/a64_timebase.rs" "$dst/src/a64_timebase.rs"
copy_frozen_source "$src/src/a64_cf_number.rs" "$dst/src/a64_cf_number.rs"
copy_frozen_source "$src/src/a64_cf_number_services.rs" "$dst/src/a64_cf_number_services.rs"
copy_frozen_source "$src/src/a64_cf_dictionary.rs" "$dst/src/a64_cf_dictionary.rs"
copy_frozen_source "$src/src/a64_commpage.rs" "$dst/src/a64_commpage.rs"
copy_frozen_source "$src/src/a64_cache_resolver.rs" "$dst/src/a64_cache_resolver.rs"
copy_frozen_source "$src/src/a64_cache_symbols.rs" "$dst/src/a64_cache_symbols.rs"
copy_frozen_source "$src/src/a64_cache_map.rs" "$dst/src/a64_cache_map.rs"
copy_frozen_source "$src/src/a64_linker.rs" "$dst/src/a64_linker.rs"
copy_frozen_source "$src/src/a64_cache_linker.rs" "$dst/src/a64_cache_linker.rs"
copy_frozen_source "$src/src/a64_cache_objc_context.rs" "$dst/src/a64_cache_objc_context.rs"
copy_frozen_source "$src/src/a64_cache_initializers.rs" "$dst/src/a64_cache_initializers.rs"
copy_frozen_source "$src/src/a64_cache_init_probe.rs" "$dst/src/a64_cache_init_probe.rs"
copy_frozen_source "$src/src/a64_commpage_ro.rs" "$dst/src/a64_commpage_ro.rs"
copy_frozen_source "$src/src/a64_mach_identity.rs" "$dst/src/a64_mach_identity.rs"
copy_frozen_source "$src/src/a64_mach_vm.rs" "$dst/src/a64_mach_vm.rs"
copy_frozen_source "$src/src/a64_mach_port_construct.rs" "$dst/src/a64_mach_port_construct.rs"
copy_frozen_source "$src/src/a64_mach_host_info.rs" "$dst/src/a64_mach_host_info.rs"
copy_frozen_source "$src/src/a64_mach_clock.rs" "$dst/src/a64_mach_clock.rs"
copy_frozen_source "$src/src/a64_mach_semaphore.rs" "$dst/src/a64_mach_semaphore.rs"
copy_frozen_source "$src/src/a64_entropy_fd.rs" "$dst/src/a64_entropy_fd.rs"
copy_frozen_source "$src/src/a64_standard_fds.rs" "$dst/src/a64_standard_fds.rs"
copy_frozen_source "$src/src/a64_mprotect.rs" "$dst/src/a64_mprotect.rs"
copy_frozen_source "$src/src/a64_mprotect_tests.rs" "$dst/src/a64_mprotect_tests.rs"
copy_frozen_source "$src/src/a64_protection_backend_tests.rs" "$dst/src/a64_protection_backend_tests.rs"
copy_frozen_source "$src/src/a64_posix_shm.rs" "$dst/src/a64_posix_shm.rs"
copy_frozen_source "$src/src/a64_main_stack.rs" "$dst/src/a64_main_stack.rs"
copy_frozen_source "$src/src/a64_native_initializer_tests.rs" "$dst/src/a64_native_initializer_tests.rs"
copy_frozen_source "$src/src/a64_thread_priority.rs" "$dst/src/a64_thread_priority.rs"
copy_frozen_source "$src/src/a64_pthread_registration.rs" "$dst/src/a64_pthread_registration.rs"
copy_frozen_source "$src/src/a64_bsdthread_ctl.rs" "$dst/src/a64_bsdthread_ctl.rs"
copy_frozen_source "$src/src/a64_objc.rs" "$dst/src/a64_objc.rs"
copy_frozen_source "$src/src/a64_objc_dispatch.rs" "$dst/src/a64_objc_dispatch.rs"
copy_frozen_source "$src/src/a64_objc_image.rs" "$dst/src/a64_objc_image.rs"
copy_frozen_source "$src/src/a64_bridge.rs" "$dst/src/a64_bridge.rs"
copy_frozen_source "$src/src/a64_objc_arc_services.rs" "$dst/src/a64_objc_arc_services.rs"
copy_frozen_source "$src/src/a64_objc_arc_services_tests.rs" "$dst/src/a64_objc_arc_services_tests.rs"
copy_frozen_source "$src/src/a64_cf.rs" "$dst/src/a64_cf.rs"
copy_frozen_source "$src/src/a64_cf_services.rs" "$dst/src/a64_cf_services.rs"
copy_frozen_source "$src/src/a64_cf_terraria_services.rs" "$dst/src/a64_cf_terraria_services.rs"
copy_frozen_source "$src/src/a64_host_services.rs" "$dst/src/a64_host_services.rs"
copy_frozen_source "$src/src/a64_startup.rs" "$dst/src/a64_startup.rs"
copy_frozen_source "$src/src/a64_pthread_tls.rs" "$dst/src/a64_pthread_tls.rs"
copy_frozen_source "$src/src/a64_pthread_tls_services.rs" "$dst/src/a64_pthread_tls_services.rs"
copy_frozen_source "$src/src/a64_pthread_mutex.rs" "$dst/src/a64_pthread_mutex.rs"
copy_frozen_source "$src/src/a64_pthread_mutex_services.rs" "$dst/src/a64_pthread_mutex_services.rs"
copy_frozen_source "$src/src/a64_pthread_cond.rs" "$dst/src/a64_pthread_cond.rs"
copy_frozen_source "$src/src/a64_pthread_cond_services.rs" "$dst/src/a64_pthread_cond_services.rs"
copy_frozen_source "$src/src/a64_objc_dealloc.rs" "$dst/src/a64_objc_dealloc.rs"
copy_frozen_source "$src/src/a64_objc_heap.rs" "$dst/src/a64_objc_heap.rs"
copy_frozen_source "$src/src/a64_objc_execution.rs" "$dst/src/a64_objc_execution.rs"
copy_frozen_source "$src/src/a64_objc_publication.rs" "$dst/src/a64_objc_publication.rs"
copy_frozen_source "$src/src/a64_objc_execution_services.rs" "$dst/src/a64_objc_execution_services.rs"
copy_frozen_source "$src/src/a64_objc_registration.rs" "$dst/src/a64_objc_registration.rs"
copy_frozen_source "$src/src/a64_objc_namespace.rs" "$dst/src/a64_objc_namespace.rs"
copy_frozen_source "$src/src/a64_objc_slots.rs" "$dst/src/a64_objc_slots.rs"
copy_frozen_source "$src/src/a64_nsstring_services.rs" "$dst/src/a64_nsstring_services.rs"
copy_frozen_source "$src/src/a64_bundle_services.rs" "$dst/src/a64_bundle_services.rs"
copy_frozen_source "$src/src/a64_bundle_load.rs" "$dst/src/a64_bundle_load.rs"
copy_frozen_source "$src/src/a64_objc_image_load.rs" "$dst/src/a64_objc_image_load.rs"
copy_frozen_source "$src/src/a64_objc_cached_root.rs" "$dst/src/a64_objc_cached_root.rs"
copy_frozen_source "$src/src/a64_foundation_startup.rs" "$dst/src/a64_foundation_startup.rs"
copy_frozen_source "$src/src/a64_pthread_create_plan.rs" "$dst/src/a64_pthread_create_plan.rs"
copy_frozen_source "$src/src/a64_pthread_create_prepare.rs" "$dst/src/a64_pthread_create_prepare.rs"
copy_frozen_source "$src/src/a64_thread_register_tests.rs" "$dst/src/a64_thread_register_tests.rs"
copy_frozen_source "$src/src/a64_bundle.rs" "$dst/src/a64_bundle.rs"
copy_frozen_source "$src/src/a64_thread_scheduler.rs" "$dst/src/a64_thread_scheduler.rs"
copy_frozen_source "$src/src/a64_thread_scheduler_cpu.rs" "$dst/src/a64_thread_scheduler_cpu.rs"
copy_frozen_source "$src/src/a64_thread_publication.rs" "$dst/src/a64_thread_publication.rs"
copy_frozen_source "$src/src/a64_scheduled_pthread_services.rs" "$dst/src/a64_scheduled_pthread_services.rs"
copy_frozen_source "$src/src/a64_cf_text.rs" "$dst/src/a64_cf_text.rs"
copy_frozen_source "$src/src/a64_cf_uuid.rs" "$dst/src/a64_cf_uuid.rs"
copy_frozen_source "$src/src/a64_cf_array.rs" "$dst/src/a64_cf_array.rs"
copy_frozen_source "$src/src/a64_cf_data.rs" "$dst/src/a64_cf_data.rs"
copy_frozen_source "$src/src/a64_objc_lifetime.rs" "$dst/src/a64_objc_lifetime.rs"
copy_frozen_source "$src/src/a64_objc_lifetime_services.rs" "$dst/src/a64_objc_lifetime_services.rs"
copy_frozen_source "$src/src/cpu/dynarmic_wrapper/a64.cpp" "$dst/src/cpu/dynarmic_wrapper/a64.cpp"
copy_frozen_source "$src/src/cpu/dynarmic_wrapper/a64.rs" "$dst/src/cpu/dynarmic_wrapper/a64.rs"
# Follow registered ARM64 modules so a newly integrated service cannot silently
# be omitted from the frozen build. Keep unchanged mtimes for incremental builds.
python3 - "$src" "$dst" <<'PY'
from pathlib import Path
import re
import shutil
import sys

source, destination = map(Path, sys.argv[1:])
pending = [Path('src/a64.rs'), Path('src/lib.rs')]
seen = set()
while pending:
    relative = pending.pop()
    if relative in seen:
        continue
    seen.add(relative)
    original = source / relative
    frozen = destination / relative
    data = original.read_bytes()
    if not frozen.is_file() or frozen.read_bytes() != data:
        frozen.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(original, frozen)
    for name in re.findall(r'#\[path\s*=\s*"([^"]+)"\]', data.decode('utf-8')):
        if name.startswith('a64_') and name.endswith('.rs'):
            pending.append(relative.parent / name)
PY
if [[ "${1:-}" == "--snapshot-only" ]]; then
    exit 0
fi
cd "$dst"
export CARGO_TARGET_DIR=/home/ben/touchHLE-a64/target
cargo test --lib
bash "$src/../tools/build_repository_ui.sh"
