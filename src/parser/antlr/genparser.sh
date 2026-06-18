#!/bin/bash
DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" >/dev/null 2>&1 && pwd )"
cd "$DIR"
java -jar ./binary/antlr4-4.8-2-SNAPSHOT-complete.jar -visitor -Dlanguage=Rust ./BuildingBlocks.g4 -o generated
rustfmt generated/buildingblockslexer.rs
rustfmt generated/buildingblocksparser.rs
rustfmt generated/buildingblockslistener.rs
rustfmt generated/buildingblocksvisitor.rs
