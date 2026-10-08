#!/usr/bin/env python3
"""Run the isolated Rust bundle model tests against actual Terraria plists.

Reuses existing host build dependency artifacts, never runs app or Apple code.
"""
import argparse
import pathlib
import subprocess
import tempfile
import zipfile
import shutil
import json

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--deps",type=pathlib.Path,default=pathlib.Path("/home/ben/touchHLE-a64/target/debug/deps"))
    parser.add_argument("--ipa",type=pathlib.Path,default=pathlib.Path("/mnt/c/Users/Ben/Desktop/ipa/Terraria_4.5.0.ipa"))
    args=parser.parse_args()
    workspace=pathlib.Path(__file__).resolve().parent.parent
    libraries=sorted(args.deps.glob("libplist-*.rlib"))
    if not libraries: raise SystemExit("Existing host plist dependency artifact required")
    with tempfile.TemporaryDirectory(prefix="a64-bundle-") as directory:
        root=pathlib.Path(directory)
        with zipfile.ZipFile(args.ipa) as ipa:
            for name,path in [("main","Payload/Terraria.app/Info.plist"),("unity","Payload/Terraria.app/Frameworks/UnityFramework.framework/Info.plist")]:
                item=ipa.getinfo(path)
                if item.file_size>1024*1024: raise ValueError("Actual plist exceeds model limit")
                (root/name).write_bytes(ipa.read(item))
        source=workspace/"touchHLE-src/src/a64_bundle.rs"
        harness=root/"harness.rs"
        quote=lambda path:json.dumps(str(path),ensure_ascii=False)
        harness.write_text(f'''#[path={quote(source)}] mod bundle;
#[test] fn actual_terraria_metadata() {{
let bytes=std::fs::read({quote(root/'main')}).unwrap();
let mut bundles=bundle::Bundles::from_main("/Terraria.app/Terraria",&bytes).unwrap();
let main=bundles.metadata(bundles.main_bundle()).unwrap();
assert_eq!(main.identifier.as_deref(),Some("com.505games.terraria"));
assert_eq!(main.executable,"Terraria"); assert_eq!(main.principal_class,None);
let unity=bundles.bundle_with_path("/Terraria.app/Frameworks/UnityFramework.framework",|_|Ok(Some(std::fs::read({quote(root/'unity')}).unwrap()))).unwrap().unwrap();
let metadata=bundles.metadata(unity).unwrap();
assert_eq!(metadata.identifier.as_deref(),Some("com.unity3d.framework"));
assert_eq!(metadata.executable,"UnityFramework");
assert_eq!(metadata.principal_class.as_deref(),Some("UnityFramework"));
assert!(!bundles.is_loaded(unity).unwrap());
}}''',encoding="utf-8")
        binary=root/"tests"
        rustc=shutil.which("rustc") or str(pathlib.Path.home()/".cargo/bin/rustc")
        subprocess.run([rustc,"--edition=2021","--test",str(harness),"--extern",f"plist={libraries[-1]}","-L",f"dependency={args.deps}","-o",str(binary)],check=True)
        subprocess.run([str(binary)],check=True)

if __name__=="__main__":main()
