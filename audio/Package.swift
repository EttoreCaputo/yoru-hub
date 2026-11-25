// swift-tools-version:5.9
// yoru-audio: Yoru Hub's helper for recording meetings (docs/meetings.md).
// Built by app/src-tauri/build.rs into app/src-tauri/binaries/, next to the app.
// YoruSync: the iCloud engine (docs/icloud.md), a static library linked into
// the app itself by the same build.rs.

import PackageDescription

let package = Package(
    name: "yoru-audio",
    // Core Audio process taps arrived in macOS 14.2; the permission prompt
    // for them in 14.4.
    platforms: [.macOS("14.4")],
    products: [
        .executable(name: "yoru-audio", targets: ["yoru-audio"]),
        .library(name: "YoruSync", type: .static, targets: ["YoruSync"]),
    ],
    dependencies: [
        // Parakeet on the Neural Engine (Apache-2.0).
        .package(url: "https://github.com/FluidInference/FluidAudio.git", exact: "0.17.7"),
    ],
    targets: [
        .executableTarget(
            name: "yoru-audio",
            dependencies: [.product(name: "FluidAudio", package: "FluidAudio")],
            path: "Sources/yoru-audio"
        ),
        .target(name: "YoruSync", path: "Sources/YoruSync"),
    ]
)
