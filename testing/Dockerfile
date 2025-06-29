# dockerfile to build and run tests in arm64
# syntax=docker/dockerfile:1.4
FROM debian:stable-slim

# install dependencies (Rust)
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    curl \
    dwarfdump \
    git \
    less \
    libssl-dev \
    pkg-config && \
    rm -rf /var/lib/apt/lists/*

# install rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

ENV PATH="/root/.cargo/bin:${PATH}"

# copy source code
WORKDIR /app

# cmd to run bash
CMD ["bash"]



