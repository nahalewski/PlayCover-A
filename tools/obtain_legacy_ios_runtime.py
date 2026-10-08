"""Acquire Apple's iPhone 6 iOS 11.4.1 firmware for old ARM64 ABI research.

Uses the existing resumable, Apple-CDN-only downloader and hash verification.
No device restoration, DRM decryption or code execution is performed.
"""
from pathlib import Path
import shutil
import hashlib
import json
import plistlib
import subprocess
import zipfile
import re
import struct

workspace = Path(__file__).resolve().parents[1]
if shutil.disk_usage(workspace).free < 20 * 1024**3:
    raise RuntimeError('Need 20 GiB free for firmware and extracted cache')
source = (workspace / 'tools' / 'obtain_ios_firmware.py').read_text()
source = source.replace("root / 'firmware'", "root / 'legacy-firmware'")
source = source.replace('iPhone10,4', 'iPhone7,2')
source = source.replace("f['version'].startswith('16.7.')", "f['version'] == '11.4.1'")
exec(compile(source, str(workspace / 'tools' / 'obtain_ios_firmware.py'), 'exec'))

assets = workspace / 'ios-runtime'
images = assets / 'legacy-images'
cache_output = assets / 'legacy-cache'
images.mkdir(exist_ok=True)
cache_output.mkdir(exist_ok=True)
with zipfile.ZipFile(destination) as archive:
    manifest = plistlib.loads(archive.read('BuildManifest.plist'))
    os_name = manifest['BuildIdentities'][0]['Manifest']['OS']['Info']['Path']
    image = images / os_name
    if not image.exists():
        print(f'Extracting original OS filesystem {os_name}', flush=True)
        archive.extract(os_name, images)
sevenzip = Path(r'C:\Program Files\7-Zip\7z.exe')
if not sevenzip.exists():
    raise RuntimeError('7-Zip with APFS/HFS support is required')
print('Extracting original ARM64 dyld shared cache without reconstructing images', flush=True)
if not any(cache_output.rglob('dyld_shared_cache_arm64')):
    subprocess.run([str(sevenzip), 'x', '-y', str(image),
                    '*dyld_shared_cache_arm64*', '-r', '-o' + str(cache_output)], check=True)
caches = sorted(cache_output.rglob('dyld_shared_cache_arm64'))
if len(caches) != 1:
    raise RuntimeError(f'Expected one original ARM64 cache, found {len(caches)}')
cache = caches[0]
ipsw = assets / 'tools' / 'ipsw.exe'
info = subprocess.run([str(ipsw), 'dyld', 'info', str(cache)],
                      capture_output=True, text=True, check=True)
(assets / 'legacy-cache-info.txt').write_text(info.stdout, encoding='utf-8')
provider = subprocess.run([str(ipsw), 'dyld', 'macho', str(cache),
                           '/usr/lib/libstdc++.6.dylib', '--loads', '--symbols'],
                          capture_output=True, text=True, check=True)
(assets / 'legacy-libstdcxx-provider.txt').write_text(provider.stdout, encoding='utf-8')
# Reuse the bounded metadata scanner, retaining its decoded import references
# for this provider-specific audit without changing the shared scanner output.
scanner = (workspace / 'tools' / 'inspect_legacy_ipa_runtime.py').read_text()
scanner = scanner.split('ipa, cache_root, output =')[0]
scanner = scanner.replace("return {\n        'opcode_counts'",
                          "return {\n        'references': sorted(references),\n        'opcode_counts'")
namespace = {}
exec(compile(scanner, 'inspect_legacy_ipa_runtime.py', 'exec'), namespace)
with zipfile.ZipFile(workspace / 'PokemonQuest-device.ipa') as archive:
    name = next(n for n in archive.namelist() if n.endswith('/CydiaSubstrate.framework/CydiaSubstrate'))
    substrate = namespace['inspect_image'](name, archive.read(name), assets / 'extractedlibs')
ordinal = next(i + 1 for i, dependency in enumerate(substrate['dependencies'])
               if dependency['name'] == '/usr/lib/libstdc++.6.dylib')
required = sorted({reference[1] for kind in ('bind', 'lazy')
                   for reference in substrate['streams'].get(kind, {}).get('references', [])
                   if reference[0] == ordinal and not reference[2]})
if not required:
    raise RuntimeError('Provider audit did not recover CydiaSubstrate imports')
present = [name for name in required if re.search(r'(?<!\w)' + re.escape(name) + r'(?!\w)', provider.stdout)]
with cache.open('rb') as handle:
    header = handle.read(0x100)
    mapping_offset, mapping_count, image_offset, image_count = struct.unpack_from('<4I', header, 16)
    handle.seek(mapping_offset)
    mappings = [struct.unpack('<QQQII', handle.read(32)) for _ in range(mapping_count)]
    def vm_offset(address):
        for base, size, offset, _, _ in mappings:
            if base <= address < base + size:
                return offset + address - base
        raise ValueError('Provider address outside original cache mappings')
    provider_address = None
    aliases = []
    for index in range(image_count):
        handle.seek(image_offset + 32 * index)
        address, _, _, path_offset, _ = struct.unpack('<QQQII', handle.read(32))
        handle.seek(path_offset)
        raw = handle.read(4096)
        path = raw[:raw.index(0)].decode()
        if path.startswith('/usr/lib/libstdc++.6'):
            aliases.append({'path': path, 'address': hex(address)})
            provider_address = address
    if provider_address is None:
        raise ValueError('libstdc++ missing from original cache image table')
    handle.seek(vm_offset(provider_address))
    macho_header = handle.read(32)
    command_count, command_bytes = struct.unpack_from('<II', macho_header, 16)
    commands = handle.read(command_bytes)
    position = 0
    export_range = None
    install_identity = None
    for _ in range(command_count):
        command, size = struct.unpack_from('<II', commands, position)
        if size < 8 or position + size > len(commands):
            raise ValueError('Malformed provider load commands')
        if command in (0x22, 0x80000022):
            export_range = struct.unpack_from('<II', commands, position + 40)
        if command == 0xd:
            name_offset = struct.unpack_from('<I', commands, position + 8)[0]
            install_identity = commands[position + name_offset:position + size].split(b'\0')[0].decode()
        position += size
    if export_range is None:
        raise ValueError('Original provider has no export trie')
    handle.seek(export_range[0])
    trie = handle.read(export_range[1])
    exports = namespace['selected_exports'](trie, required)
audit = {'consumer': substrate['path'], 'provider': '/usr/lib/libstdc++.6.dylib',
         'required_symbols': required, 'present_symbols': present,
         'missing_symbols': [name for name, entry in exports.items() if entry.get('absent')],
         'exports': exports, 'install_identity': install_identity, 'cache_table_aliases': aliases,
         'architecture': 'ARM64', 'mapping_offset': mapping_offset,
         'slide_info_version': 2,
         'scope': 'Original-cache export trie and symbol inventory; binding and execution still require validation',
         'execution_unsupported': True}
(assets / 'legacy-libstdcxx-symbol-audit.json').write_text(json.dumps(audit, indent=2) + '\n')
def sha256(path):
    with path.open('rb') as handle:
        return hashlib.file_digest(handle, 'sha256').hexdigest()
provenance = {
    'firmware': firmware, 'metadata_url': metadata_url,
    'os_image': str(image.relative_to(workspace)), 'os_image_sha256': sha256(image),
    'cache': str(cache.relative_to(workspace)), 'cache_sha256': sha256(cache),
    'cache_size': cache.stat().st_size, 'provider': '/usr/lib/libstdc++.6.dylib',
    'tools': {'7zip': '26.03', 'ipsw': '3.1.731'},
    'original_cache_preserved': True, 'execution_unsupported': True,
}
(assets / 'legacy-provider-provenance.json').write_text(
    json.dumps(provenance, indent=2) + '\n', encoding='utf-8')
print(f'Original legacy cache preserved: {cache}', flush=True)
