FROM --platform=$BUILDPLATFORM ghcr.io/astral-sh/uv:0.13.0@sha256:cdc6093146eb3ff6a40107b38f008b789e050e77ad87865e381d9917da55a168 AS uv

FROM --platform=$BUILDPLATFORM ubuntu:24.04@sha256:68ee22004a9109b44973313b02ff7b1de250f1de0f5ecb628d73f579aaff33ac AS build-tools
ENV HOME="/root"
WORKDIR $HOME

RUN apt update && apt install -y build-essential curl

# Install uv
COPY --from=uv /uv /usr/local/bin/uv

# Setup zig as cross compiling linker
COPY pyproject.toml uv.lock ./
RUN uv sync --only-group docker --locked
ENV PATH="$HOME/.venv/bin:$PATH"

FROM build-tools AS build
WORKDIR $HOME/serc

# Install rust
ARG TARGETPLATFORM
RUN case "$TARGETPLATFORM" in \
    "linux/arm64") echo "aarch64-unknown-linux-musl" > rust_target.txt ;; \
    "linux/amd64") echo "x86_64-unknown-linux-musl" > rust_target.txt ;; \
    *) exit 1 ;; \
    esac
# Update rustup whenever we bump the rust version
COPY rust-toolchain.toml rust-toolchain.toml
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --target $(cat rust_target.txt) --profile minimal --default-toolchain none
ENV PATH="$HOME/.cargo/bin:$PATH"
# Installs the correct toolchain version from rust-toolchain.toml and then the musl target
RUN rustup target add $(cat rust_target.txt)

# Build
COPY crates crates
COPY Cargo.toml Cargo.lock ./
RUN cargo zigbuild --bin serc --target $(cat rust_target.txt) --release --locked
RUN cp target/$(cat rust_target.txt)/release/serc /serc

FROM scratch
COPY --from=build /serc /serc
WORKDIR /io
ENTRYPOINT ["/serc"]
