"""Test linker-generated chained and classic dyld imports through file/IPA launches."""
import pathlib
import plistlib
import shutil
import subprocess
import sys
import tempfile
import zipfile

binary = pathlib.Path(sys.argv[1]).resolve()
fixtures = pathlib.Path(sys.argv[2]).resolve()


def launch(args, cwd):
    result = subprocess.run([str(binary), *args], cwd=cwd, capture_output=True,
                            text=True, timeout=30)
    return result, result.stdout + result.stderr


with tempfile.TemporaryDirectory(prefix="playcover-a64-link-") as folder:
    folder = pathlib.Path(folder)
    frameworks = folder / "Frameworks"
    frameworks.mkdir()
    client = folder / "ImportClient"
    shutil.copyfile(fixtures / "import_client.macho", client)
    provider = frameworks / "libAnswer.dylib"
    shutil.copyfile(fixtures / "libAnswer.dylib", provider)
    result, output = launch([f"--a64-run={client}"], folder)
    assert result.returncode == 0 and "ARM64 guest exited with code 42" in output, output

    ipa = folder / "ImportClient.ipa"
    info = {"CFBundleIdentifier": "local.playcover.importclient",
            "CFBundleName": "ImportClient", "CFBundleExecutable": "ImportClient",
            "CFBundleVersion": "1", "MinimumOSVersion": "14.0"}
    with zipfile.ZipFile(ipa, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("Payload/ImportClient.app/Info.plist", plistlib.dumps(info))
        archive.write(client, "Payload/ImportClient.app/ImportClient")
        archive.write(provider, "Payload/ImportClient.app/Frameworks/libAnswer.dylib")
    result, output = launch([str(ipa), "--headless"], folder)
    assert result.returncode == 0 and "ARM64 guest exited with code 42" in output, output

    # Keep the dependency but remove the requested export from both trie/symtab.
    original = provider.read_bytes()
    provider.write_bytes(original.replace(b"_answer", b"_absent"))
    result, output = launch([f"--a64-run={client}"], folder)
    assert result.returncode != 0 and "_answer" in output, output
    assert "panic" not in output.lower(), output
    provider.write_bytes(original)

    provider.unlink()
    result, output = launch([f"--a64-run={client}"], folder)
    assert result.returncode != 0 and "libAnswer.dylib" in output, output
    assert "panic" not in output.lower(), output
    shutil.copyfile(fixtures / "libInitialized.dylib", frameworks / "libInitialized.dylib")
    shutil.copyfile(fixtures / "initializer_client.macho", client)
    result, output = launch([f"--a64-run={client}"], folder)
    assert result.returncode == 0 and "ARM64 guest exited with code 42" in output, output
    shutil.copyfile(fixtures / "sparse_client.macho", client)
    result, output = launch([f"--a64-run={client}"], folder)
    assert result.returncode == 0 and "ARM64 guest exited with code 42" in output, output

    # Older iOS images use opcode-based relocation streams rather than chained
    # fixups. The provider's data pointer requires a slide rebase. The two
    # clients exercise ordinary GOT binding and eager resolution of lazy slots;
    # the latter must bypass the provider's dyld_stub_binder sentinel (99).
    legacy_provider = frameworks / "libLegacyAnswer.dylib"
    shutil.copyfile(fixtures / legacy_provider.name, legacy_provider)
    for stem in ("legacy_client", "legacy_lazy_client"):
        legacy_client = folder / stem
        shutil.copyfile(fixtures / (stem + ".macho"), legacy_client)
        result, output = launch([f"--a64-run={legacy_client}"], folder)
        assert result.returncode == 0 and "ARM64 guest exited with code 42" in output, output
        legacy_ipa = folder / (stem + ".ipa")
        shutil.copyfile(fixtures / (stem + ".ipa"), legacy_ipa)
        result, output = launch([str(legacy_ipa), "--headless"], folder)
        assert result.returncode == 0 and "ARM64 guest exited with code 42" in output, output

    original = legacy_provider.read_bytes()
    legacy_provider.write_bytes(original.replace(b"_answer", b"_absent"))
    result, output = launch([f"--a64-run={folder / 'legacy_lazy_client'}"], folder)
    assert result.returncode != 0 and "_answer" in output, output
    assert "panic" not in output.lower(), output
    legacy_provider.write_bytes(original)
    runtime_client = folder / "runtime_services"
    shutil.copyfile(fixtures / "runtime_services.macho", runtime_client)
    result, output = launch([f"--a64-run={runtime_client}"], folder)
    assert result.returncode == 0 and "ARM64 guest exited with code 42" in output, output
    runtime_ipa = folder / "runtime_services.ipa"
    shutil.copyfile(fixtures / "runtime_services.ipa", runtime_ipa)
    result, output = launch([str(runtime_ipa), "--headless"], folder)
    assert result.returncode == 0 and "ARM64 guest exited with code 42" in output, output
    initializer_ipa = folder / "runtime_initializer.ipa"
    shutil.copyfile(fixtures / "runtime_initializer.ipa", initializer_ipa)
    result, output = launch([str(initializer_ipa), "--headless"], folder)
    assert result.returncode == 0 and "ARM64 guest exited with code 42" in output, output
    print("ARM64 chained/classic linking, runtime services and initializer file/IPA launches passed")
