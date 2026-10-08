#!/bin/bash
# Build LLVM 3.4.2 libc++ as an armv7 iOS dylib (the C++ library iOS 5-7 apps were built against),
# with touchHLE's common-3.0 SDK. libc++abi symbols (cxa_*, typeinfo) are left to the app's libstdc++/libgcc.
cd ~/libcxx-build
SDK=$HOME/libcxx-build/sdk/common-3.0.sdk
SRC=$HOME/libcxx-build/llvm34/libcxx
OUT=$HOME/libcxx-build/out34
mkdir -p $OUT
CXXFLAGS="--target=armv7-apple-ios3.0 --sysroot=$SDK -isysroot $SDK -B$SDK/usr/bin -Wno-incompatible-sysroot -mlinker-version=253 -mfpu=vfpv3 \
  -std=c++11 -O2 -fPIC -nostdinc++ -I$SRC/include -isystem $HOME/libcxx-build/extra-include \
  -include /home/ben/libcxx-build/ios_shim.h -D_LIBCPP_BUILDING_LIBRARY -D__STDC_FORMAT_MACROS -D__STDC_LIMIT_MACROS -D__STDC_CONSTANT_MACROS \
  -Wno-deprecated-declarations -Wno-everything"
rm -f $OUT/*.o $OUT/errors.log
ls $SRC/src/*.cpp | xargs -P 16 -I{} sh -c 'f={}; b=$(basename $f .cpp); clang++ '"$CXXFLAGS"' -c $f -o '"$OUT"'/$b.o 2> '"$OUT"'/$b.err || echo "FAIL $b" >> '"$OUT"'/errors.log'
echo "compiled: $(ls $OUT/*.o 2>/dev/null | wc -l) / $(ls $SRC/src/*.cpp | wc -l)"
cat $OUT/errors.log 2>/dev/null
