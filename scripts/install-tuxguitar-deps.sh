#!/bin/sh
# Install TuxGuitar 2.0.1 jars from an installed TuxGuitar.app into the local
# Maven repository, so the bridge plugin can compile against them.
#
# Usage: scripts/install-tuxguitar-deps.sh [/path/to/TuxGuitar.app]
set -eu

APP="${1:-/Applications/tuxguitar-2.0.1-macosx-swt-cocoa-x86_64.app}"
LIB="$APP/Contents/MacOS/lib"
VERSION="2.0.1"

if [ ! -d "$LIB" ]; then
    echo "error: $LIB not found — pass the TuxGuitar.app path as the first argument" >&2
    exit 1
fi

for artifact in tuxguitar-lib tuxguitar-editor-utils tuxguitar-ui-toolkit tuxguitar; do
    jar="$LIB/$artifact.jar"
    if [ ! -f "$jar" ]; then
        echo "error: $jar not found" >&2
        exit 1
    fi
    echo "installing app.tuxguitar:$artifact:$VERSION from $jar"
    # -DgeneratePom=true: ignore the POM embedded in the release jar — it
    # references the tuxguitar-pom parent, which exists in no repository.
    mvn -q install:install-file \
        -Dfile="$jar" \
        -DgroupId=app.tuxguitar \
        -DartifactId="$artifact" \
        -Dversion="$VERSION" \
        -Dpackaging=jar \
        -DgeneratePom=true
done

echo "done"
