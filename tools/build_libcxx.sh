#!/bin/bash
# Cross-build LLVM libc++ (+ libc++abi folded in) as an ARM iOS dylib with touchHLE's common-3.0 SDK.
cd ~/libcxx-build
SDK=$HOME/libcxx-build/sdk/common-3.0.sdk
COMMON="-B$SDK/usr/bin -Wno-incompatible-sysroot -mlinker-version=253 -mfpu=vfpv3 -isystem $HOME/libcxx-build/extra-include -isystem $HOME/libcxx-build/llvm-project/libunwind/include"
echo start
cmake -G Ninja -S llvm-project/runtimes -B build \
  -DCMAKE_SYSTEM_NAME=Darwin -DCMAKE_SYSTEM_PROCESSOR=arm \
  -DCMAKE_SYSROOT=$SDK -DCMAKE_OSX_SYSROOT=$SDK -DCMAKE_OSX_DEPLOYMENT_TARGET= -DCMAKE_OSX_ARCHITECTURES= \
  -DCMAKE_TRY_COMPILE_TARGET_TYPE=STATIC_LIBRARY \
  -DCMAKE_AR=/usr/bin/llvm-ar -DCMAKE_RANLIB=/usr/bin/llvm-ranlib \
  -DCMAKE_C_COMPILER=clang -DCMAKE_CXX_COMPILER=clang++ \
  -DCMAKE_C_COMPILER_TARGET=armv7-apple-ios3.0 -DCMAKE_CXX_COMPILER_TARGET=armv7-apple-ios3.0 \
  -DCMAKE_BUILD_TYPE=Release \
  "-DCMAKE_C_FLAGS=$COMMON" "-DCMAKE_CXX_FLAGS=$COMMON -fno-aligned-allocation -D_LIBCPP_HAS_NO_LIBRARY_ALIGNED_ALLOCATION" \
  "-DCMAKE_SHARED_LINKER_FLAGS=-Wl,-install_name,/usr/lib/libc++.1.dylib -Wl,-compatibility_version,1.0.0 -Wl,-current_version,307.4.0" \
  -DLLVM_ENABLE_RUNTIMES="libcxx;libcxxabi" \
  -DLIBCXX_ENABLE_SHARED=ON -DLIBCXX_ENABLE_STATIC=OFF \
  -DLIBCXXABI_ENABLE_SHARED=OFF -DLIBCXXABI_ENABLE_STATIC=ON \
  -DLIBCXX_CXX_ABI=libcxxabi -DLIBCXX_STATICALLY_LINK_ABI_IN_SHARED_LIBRARY=ON \
  -DLIBCXXABI_USE_LLVM_UNWINDER=OFF -DLIBCXX_ABI_VERSION=1 \
  -DLIBCXX_ENABLE_EXCEPTIONS=ON -DLIBCXXABI_ENABLE_EXCEPTIONS=ON \
  -DLIBCXX_INCLUDE_TESTS=OFF -DLIBCXX_INCLUDE_BENCHMARKS=OFF -DLIBCXXABI_INCLUDE_TESTS=OFF \
  -DLIBCXX_ENABLE_FILESYSTEM=OFF -DLIBCXX_ENABLE_RANDOM_DEVICE=OFF > ~/libcxx-build/configure.log 2>&1
echo "configure exit: $?"
ninja -C build cxx > ~/libcxx-build/ninja.log 2>&1
echo "ninja exit: $?"
