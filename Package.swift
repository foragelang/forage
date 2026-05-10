// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "Forage",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "Forage", targets: ["Forage"]),
        .executable(name: "forage-probe", targets: ["forage-probe"]),
    ],
    targets: [
        .target(
            name: "Forage",
            path: "Sources/Forage"
        ),
        .executableTarget(
            name: "forage-probe",
            dependencies: ["Forage"],
            path: "Sources/forage-probe"
        ),
        .testTarget(
            name: "ForageTests",
            dependencies: ["Forage"],
            path: "Tests/ForageTests"
        ),
    ]
)
