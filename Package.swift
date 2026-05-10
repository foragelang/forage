// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "Forage",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "Forage", targets: ["Forage"]),
    ],
    targets: [
        .target(
            name: "Forage",
            path: "Sources/Forage"
        ),
        .testTarget(
            name: "ForageTests",
            dependencies: ["Forage"],
            path: "Tests/ForageTests"
        ),
    ]
)
