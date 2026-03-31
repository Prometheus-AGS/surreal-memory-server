docker compose up -d --build
[+] Building 255.0s (25/25) FINISHED                                                                                                                                                     
 => [internal] load local bake definitions                                                                                                                                          0.0s
 => => reading from stdin 753B                                                                                                                                                      0.0s
 => [internal] load build definition from Dockerfile                                                                                                                                0.0s
 => => transferring dockerfile: 2.35kB                                                                                                                                              0.0s
 => resolve image config for docker-image://docker.io/docker/dockerfile:1.4                                                                                                         0.4s
 => [auth] docker/dockerfile:pull token for registry-1.docker.io                                                                                                                    0.0s
 => CACHED docker-image://docker.io/docker/dockerfile:1.4@sha256:9ba7531bd80fb0a858632727cf7a112fbfd19b17e94c4e84ced81e24ef1a0dbc                                                   0.0s
 => => resolve docker.io/docker/dockerfile:1.4@sha256:9ba7531bd80fb0a858632727cf7a112fbfd19b17e94c4e84ced81e24ef1a0dbc                                                              0.0s
 => [internal] load .dockerignore                                                                                                                                                   0.0s
 => => transferring context: 488B                                                                                                                                                   0.0s
 => [internal] load metadata for docker.io/library/rust:1.93-slim                                                                                                                   0.0s
 => [internal] load metadata for docker.io/library/debian:trixie-slim                                                                                                               0.3s
 => [auth] library/debian:pull token for registry-1.docker.io                                                                                                                       0.0s
 => CACHED [builder 1/8] FROM docker.io/library/rust:1.93-slim@sha256:7e6fa79cf81be23fd45d857f75f583d80cfdbb11c91fa06180fd747fda37a61d                                              0.0s
 => => resolve docker.io/library/rust:1.93-slim@sha256:7e6fa79cf81be23fd45d857f75f583d80cfdbb11c91fa06180fd747fda37a61d                                                             0.0s
 => CACHED [runtime 1/5] FROM docker.io/library/debian:trixie-slim@sha256:26f98ccd92fd0a44d6928ce8ff8f4921b4d2f535bfa07555ee5d18f61429cf0c                                          0.0s
 => => resolve docker.io/library/debian:trixie-slim@sha256:26f98ccd92fd0a44d6928ce8ff8f4921b4d2f535bfa07555ee5d18f61429cf0c                                                         0.0s
 => [internal] load build context                                                                                                                                                   0.2s
 => => transferring context: 85.19MB                                                                                                                                                0.2s
 => [builder 2/8] RUN apt-get update && apt-get install -y     clang     libclang-dev     lld     pkg-config     build-essential     cmake     && rm -rf /var/lib/apt/lists/*      20.5s
 => [runtime 2/5] RUN apt-get update     && apt-get install -y --no-install-recommends ca-certificates curl libssl3 libstdc++6     && rm -rf /var/lib/apt/lists/*                   8.7s
 => [builder 3/8] WORKDIR /app                                                                                                                                                      0.1s
 => [builder 4/8] COPY Cargo.toml Cargo.lock ./                                                                                                                                     0.0s
 => [builder 5/8] COPY crates/surreal-memory/Cargo.toml crates/surreal-memory/                                                                                                      0.0s
 => [builder 6/8] RUN if [ "1" = "1" ]; then       mkdir -p src crates/surreal-memory/src       && echo "fn main() {}" > src/main.rs       && echo "" > crates/surreal-memory/sr  220.0s
 => [builder 7/8] COPY . .                                                                                                                                                          0.1s
 => [builder 8/8] RUN touch src/main.rs crates/surreal-memory/src/lib.rs     && cargo build --release --bin surreal-memory-server --no-default-features --features server-only     12.2s
 => [runtime 3/5] COPY --from=builder /app/target/release/surreal-memory-server /usr/local/bin/surreal-memory-server                                                                0.0s
 => [runtime 4/5] RUN useradd -m -u 1001 smserver                                                                                                                                   0.1s
 => [runtime 5/5] WORKDIR /data                                                                                                                                                     0.0s
 => exporting to image                                                                                                                                                              0.8s
 => => exporting layers                                                                                                                                                             0.6s
 => => exporting manifest sha256:d88c7c4251cd4d62212d076faecb19b37db529e8a8ad192c8d42e558192cb59c                                                                                   0.0s
 => => exporting config sha256:68848ab09d73e12d8706425fbdef274392e562c2fdf5102d720770714c679948                                                                                     0.0s
 => => exporting attestation manifest sha256:ed5214b674b2145af5e6400e56426fe4192b026ea4a7d9926e9f4ca673186b03                                                                       0.0s
 => => exporting manifest list sha256:c5a7c60cdaa61f2905ad5274496c406e309cacf8d7761c40de3aa721b9f7c62c                                                                              0.0s
 => => naming to docker.io/library/surreal-memory-server-surreal-memory-server:latest                                                                                               0.0s
 => => unpacking to docker.io/library/surreal-memory-server-surreal-memory-server:latest                                                                                            0.2s
 => resolving provenance for metadata file                                                                                                                                          0.0s
[+] up 3/3
 ✔ Image surreal-memory-server-surreal-memory-server Built                                                                                                                         255.1s
 ✘ Container surrealdb                               Error dependency surrealdb failed to start                                                                                    67.3s
 ✔ Container surreal-memory-server                   Recreated                                                                                                                     10.3s
dependency failed to start: container surrealdb is unhealthy