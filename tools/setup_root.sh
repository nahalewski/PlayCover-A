#!/bin/bash
set -e
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq unzip zip openjdk-17-jdk-headless build-essential pkg-config rsync wget ca-certificates
java -version 2>&1 | head -1
