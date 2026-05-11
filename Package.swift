// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "Forage",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "Forage", targets: ["Forage"]),
        .executable(name: "forage", targets: ["forage-cli"]),
    ],
    dependencies: [
        .package(url: "https://github.com/apple/swift-argument-parser", from: "1.3.0"),
        .package(url: "https://github.com/scinfu/SwiftSoup", from: "2.7.0"),
    ],
    targets: [
        .target(
            name: "Forage",
            dependencies: [
                .product(name: "SwiftSoup", package: "SwiftSoup"),
            ],
            path: "Sources/Forage"
        ),
        .executableTarget(
            name: "forage-cli",
            dependencies: [
                "Forage",
                .product(name: "ArgumentParser", package: "swift-argument-parser"),
            ],
            path: "Sources/forage-cli"
        ),
        .testTarget(
            name: "ForageTests",
            dependencies: ["Forage"],
            path: "Tests/ForageTests"
        ),
    ]
)
