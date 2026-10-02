// swift-tools-version: 5.9
// The swift-tools-version declares the minimum version of Swift required to build this package.

import PackageDescription

let package = Package(
    name: "SugargliderUI",
    platforms: [
        .macOS(.v13)
    ],
    products: [
        .library(
            name: "SugargliderUI",
            type: .dynamic,
            targets: ["SugargliderUI"]
        ),
    ],
    targets: [
        .target(
            name: "SugargliderUI",
            swiftSettings: [
                .enableExperimentalFeature("StrictConcurrency")
            ],
            linkerSettings: [
                // Allow undefined symbols - they will be provided by the Rust binary at runtime
                .unsafeFlags(["-Xlinker", "-undefined", "-Xlinker", "dynamic_lookup"])
            ]
        ),
        .testTarget(
            name: "SugargliderUITests",
            dependencies: ["SugargliderUI"]
        ),
    ]
)
