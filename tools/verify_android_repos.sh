#!/bin/bash
set -euo pipefail
source ~/.cargo/env
export JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64
export ANDROID_HOME=/home/ben/android-sdk
cd /home/ben/touchHLE-a64-integration/android
/home/ben/gradle-8.11.1/bin/gradle --no-daemon :app:testDebugUnitTest -PEXCLUDE_NATIVE_LIBS=true
