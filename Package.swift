// swift-tools-version:6.0
//
// OccipitalKit: the Swift package an app pins to embed occipital-lang.
//
// OccipitalCore is the prebuilt static library (crates/occipital-ffi, built
// for iPhone and the iPhone simulator by scripts/build-xcframework.sh) with
// its C header. OccipitalKit is the safe Swift wrapper over that C ABI.

import PackageDescription

let package = Package(
    name: "OccipitalKit",
    // The archives in the xcframework are built for iOS 17.0 and later.
    platforms: [.iOS(.v17)],
    products: [
        .library(name: "OccipitalKit", targets: ["OccipitalKit"])
    ],
    targets: [
        .binaryTarget(
            name: "OccipitalCore",
            path: "ports/swift/OccipitalCore.xcframework"
        ),
        .target(
            name: "OccipitalKit",
            dependencies: ["OccipitalCore"],
            path: "ports/swift/Sources/OccipitalKit"
        ),
    ],
    swiftLanguageModes: [.v6]
)
