#!/bin/bash
# Finish the LLVM 3.4.2 libc++ build: the two files that need C++03 mode, then link the dylib.
cd ~/libcxx-build
SDK=$HOME/libcxx-build/sdk/common-3.0.sdk
SRC=$HOME/libcxx-build/llvm34/libcxx
OUT=$HOME/libcxx-build/out34
for b in hash debug; do
  clang++ --target=armv7-apple-ios3.0 --sysroot=$SDK -isysroot $SDK -B$SDK/usr/bin -Wno-incompatible-sysroot \
    -mlinker-version=253 -mfpu=vfpv3 -std=c++03 -O2 -fPIC -nostdinc++ -I$SRC/include \
    -isystem $HOME/libcxx-build/extra-include -include $HOME/libcxx-build/ios_shim.h \
    -D_LIBCPP_BUILDING_LIBRARY -Wno-everything -c $SRC/src/$b.cpp -o $OUT/$b.o 2> $OUT/$b.err \
    && echo "ok $b" || { echo "FAIL $b"; grep -m3 error $OUT/$b.err | cut -c1-200; }
done
echo "objects: $(ls $OUT/*.o | wc -l)"
clang++ --target=armv7-apple-ios3.0 --sysroot=$SDK -isysroot $SDK -B$SDK/usr/bin -Wno-incompatible-sysroot \
  -mlinker-version=253 -dynamiclib -nodefaultlibs -nostdlib++ \
  -Wl,-install_name,/usr/lib/libc++.1.dylib -Wl,-compatibility_version,1.0.0 -Wl,-current_version,120.0.0 \
  -Wl,-image_base,0x37800000 -Wl,-undefined,dynamic_lookup -o $OUT/libc++.1.dylib $OUT/*.o -lSystem 2> $OUT/link.err
echo "link exit: $?"
head -20 $OUT/link.err | cut -c1-220
ls -la $OUT/libc++.1.dylib 2>&1
