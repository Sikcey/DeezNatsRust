# Third-party notices

DeezNats Rust is an unofficial Deezer client, not affiliated with or endorsed by Deezer.
Service names, content and artwork remain the property of their respective owners.

## DeezNats and OpenDeezer

The Rust edition adapts the interface, localization catalogs and playback behavior of
[DeezNats](https://github.com/Sikcey/DeezNats), licensed under AGPL-3.0.
The external Go playback engine derives from
[Cycl0o0/OpenDeezer](https://github.com/Cycl0o0/OpenDeezer), upstream commit
`8c86ea179ac622c4ac03fc41e6dcacd228fdc492` (v3.1.4), with local modifications.
Engine provenance, source and licenses remain in the original project's
[vendor/opendeezer](https://github.com/Sikcey/DeezNats/tree/main/vendor/opendeezer).
The engine is not vendored here. Distributors must provide the corresponding
modified engine source and preserve its notices when shipping its DLL.
The application license is retained at [LICENSE](LICENSE).

## Build and runtime dependencies

Rust dependency versions are recorded in `Cargo.lock`; each crate retains its own
license. Windows bindings use the Microsoft windows and windows-sys crates
(MIT OR Apache-2.0); libloading uses ISC. WebView2 is subject to Microsoft's
applicable SDK and runtime terms. NSIS builds the Windows installer.

## Connect MP3 streaming

Browser streams use unmodified [LAME 3.100](https://lame.sourceforge.io/),
statically built by [mp3lame-sys 0.1.11](https://crates.io/crates/mp3lame-sys/0.1.11)
through [mp3lame-encoder 0.2.5](https://crates.io/crates/mp3lame-encoder/0.2.5).
The Rust bindings are LGPL-3.0; LAME is LGPL-2.0-or-later. License copies are
retained in [mp3lame-LGPL-3.0.txt](assets/licenses/mp3lame-LGPL-3.0.txt) and
[lame-LGPL-2.0.txt](assets/licenses/lame-LGPL-2.0.txt), included in the corresponding
source archive. The pinned mp3lame-sys source package includes the complete LAME
source used by its build script. The application's source and Cargo build files
permit rebuilding/relinking with modified libraries. Decoder regression tests use
[Symphonia](https://github.com/pdeljanov/Symphonia) (MPL-2.0), a test-only dependency.

## Binary update patches

Binary deltas use [qbsdiff](https://github.com/hucsmn/qbsdiff) under the MIT
license. Its copyright and permission notice is retained in
[assets/licenses/qbsdiff.txt](assets/licenses/qbsdiff.txt), included in source
distributions and installed as `qbsdiff.LICENSE`. RustCrypto SHA-256 and semver
are MIT OR Apache-2.0; exact dependency versions remain in `Cargo.lock`.

## KuGou protocol reference

The KuGou provider and KRC parser use
[f1nniboy/lrcmux](https://github.com/f1nniboy/lrcmux/tree/main/internal/providers/kugou)
as a protocol reference. Its MIT notice follows:

Copyright © 2026 f1nniboy

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the “Software”), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED “AS IS”, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## Provider logos

Deezer logo outlines come from [Simple Icons](https://github.com/simple-icons/simple-icons),
distributed under CC0-1.0. SVG sources are retained in `assets/providers`.
The Deezer mark belongs to its owner.

Dependency licenses are not replaced by this project's license.

## YouTube Music

The YouTube Music adapter and its `ytmusic-rs` dependency are GPL-3.0-or-later.
The locally modified dependency is retained in `vendor/ytmusic`, including its
license and upstream revision in `README.local.md`.
The license text is retained in
[assets/licenses/sonora-GPL-3.0.txt](assets/licenses/sonora-GPL-3.0.txt).
The private login window is adapted from this application's Deezer login window.

## Sonora lyric effects

The native lyric renderer adapts the distance-based verse haze curve, blur
quantization, spring scrolling and row lag, viewport pinning and fading, and
past/upcoming opacity values from
[Sonora](https://github.com/sonorahq/sonora),
`crates/views/src/chrome/aside.rs` (GPL-3.0-or-later).
The license is retained in
[assets/licenses/sonora-GPL-3.0.txt](assets/licenses/sonora-GPL-3.0.txt).
