// swift-tools-version: 6.1

import PackageDescription

let package = Package(
    name: "surreal-memory-mlx-executor",
    platforms: [.macOS(.v14)],
    products: [
        .executable(
            name: "surreal-memory-mlx-executor",
            targets: ["SurrealMemoryMLXExecutor"]
        )
    ],
    dependencies: [
        .package(
            url: "https://github.com/ml-explore/mlx-swift-lm",
            exact: "3.31.4"
        ),
        .package(
            url: "https://github.com/ml-explore/mlx-swift",
            exact: "0.31.4"
        ),
        .package(
            url: "https://github.com/huggingface/swift-huggingface",
            exact: "0.9.0"
        ),
        .package(
            url: "https://github.com/huggingface/swift-transformers",
            exact: "1.3.0"
        )
    ],
    targets: [
        .target(name: "SurrealMemoryMLXExecutorCore"),
        .executableTarget(
            name: "SurrealMemoryMLXExecutor",
            dependencies: [
                "SurrealMemoryMLXExecutorCore",
                .product(name: "MLX", package: "mlx-swift"),
                .product(name: "MLXEmbedders", package: "mlx-swift-lm"),
                .product(name: "MLXLMCommon", package: "mlx-swift-lm"),
                .product(name: "MLXHuggingFace", package: "mlx-swift-lm"),
                .product(name: "HuggingFace", package: "swift-huggingface"),
                .product(name: "Tokenizers", package: "swift-transformers")
            ]
        ),
        .testTarget(
            name: "SurrealMemoryMLXExecutorCoreTests",
            dependencies: ["SurrealMemoryMLXExecutorCore"]
        )
    ]
)
