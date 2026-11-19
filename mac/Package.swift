// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "Rotae",
    platforms: [.macOS(.v14)],
    products: [.executable(name: "Rotae", targets: ["Rotae"])],
    targets: [
        .target(name: "SDCore", path: "Sources/SDCore"),
        .executableTarget(name: "Rotae", dependencies: ["SDCore"], path: "Sources/Rotae"),
        .executableTarget(name: "sd-native", path: "Sources/sd-native"),
        .executableTarget(name: "rotae-cli", dependencies: ["SDCore"], path: "Sources/rotae-cli"),
        .executableTarget(name: "sd-selftest", dependencies: ["SDCore"], path: "Sources/sd-selftest"),
    ]
)
