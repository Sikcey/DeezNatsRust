# Source and dependencies

DeezNats Rust and DeezNats Connect are distributed under AGPL-3.0. See LICENSE
and NOTICE.md for the application license and third-party notices.

The accompanying DeezNats-Source.zip contains the Rust application and Connect
client source, Cargo.lock, build scripts, and the modified OpenDeezer engine
source used for the bundled libdeezercore.dll. Local uncommitted application
changes are included so the archive matches this package.

Build instructions are in DeezNatsRust/docs/BUILDING.md inside the archive.
Rust dependencies are pinned in DeezNatsRust/Cargo.lock; Go dependencies are
pinned in DeezerNative/vendor/opendeezer/go.mod and go.sum. Their respective
licenses and the engine's retained notices continue to apply.

Project repositories:

- https://github.com/Sikcey/DeezNatsRust
- https://github.com/Sikcey/DeezNats

User profiles, credentials, downloaded media, and private development files are
not included in the source archive.
