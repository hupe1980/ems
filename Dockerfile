FROM rust:1.99-bookworm@sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c AS build
RUN apt-get update && apt-get install -y --no-install-recommends cmake libclang-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY services services
COPY tools/ems-wasm tools/ems-wasm
COPY xtask xtask
ARG TARGETARCH
RUN --mount=type=cache,id=ems-registry-${TARGETARCH},target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=ems-target-${TARGETARCH},target=/src/target,sharing=locked \
    cargo build --locked --release --bin hemsd --bin tariffd && \
    mkdir -p /out/state && cp target/release/hemsd target/release/tariffd /out/

RUN awk '/^name = "serialport"$/{getline; gsub(/"/, "", $3); print "serialport " $3 " (MPL-2.0)"; print "Unmodified source: https://static.crates.io/crates/serialport/serialport-" $3 ".crate"}' Cargo.lock > /out/serialport.txt && test -s /out/serialport.txt

FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f AS runtime
COPY LICENSE-MIT LICENSE-APACHE /usr/share/doc/ems/

FROM runtime AS hemsd
COPY --from=build /out/serialport.txt /usr/share/doc/ems/serialport.txt
COPY --from=build /out/hemsd /usr/local/bin/hemsd
COPY --from=build --chown=65532:65532 /out/state /var/lib/hemsd
WORKDIR /var/lib/hemsd
ENTRYPOINT ["/usr/local/bin/hemsd"]
CMD ["run", "--config", "/etc/ems/hemsd.toml"]

FROM runtime AS tariffd
COPY --from=build /out/tariffd /usr/local/bin/tariffd
ENTRYPOINT ["/usr/local/bin/tariffd"]
CMD ["--config", "/etc/ems/tariffd.toml"]
