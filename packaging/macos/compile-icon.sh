#!/bin/sh
# Nivra macOS icon is a prebuilt raster ICNS (resvg, no Icon Composer layers yet).
# Keep actool wiring for a future Nivra.icon set with light/dark variants.
set -eu
resources="$1"
mkdir -p "$resources"
cp packaging/macos/Nivra.icns "$resources/Nivra.icns"
