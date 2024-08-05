# Runs on openSUSE
#
# docker build -t wild-dev-opensuse . -f docker/opensuse.Dockerfile
# docker run -it wild-dev-opensuse

FROM opensuse/tumbleweed@sha256:01b3ff6b39bf9112c7e8e0fccdd130f2b557149b9c3cd806d60a24716acc377d AS chef
RUN zypper install -y -t pattern devel_C_C++ && \
    zypper install -y \
        rustup \
        clang \
        glibc-devel-static \
        lld \
        vim \
        less \
        git \
        cmake make \
        tbb-devel \
        libzstd-devel
RUN rustup toolchain install nightly
RUN cargo install --locked cargo-chef
RUN rustup target add x86_64-unknown-linux-musl && \
    rustup target add x86_64-unknown-linux-musl --toolchain nightly && \
    rustup component add rustc-codegen-cranelift-preview --toolchain nightly
WORKDIR /wild

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /wild/recipe.json recipe.json
RUN cargo chef cook --all-targets --recipe-path recipe.json
COPY . .
RUN git clone https://github.com/marxin/mold.git /mold
WORKDIR /mold
RUN git checkout origin/x86_64-only
RUN mkdir build
RUN mkdir build-wild
WORKDIR /mold/build
RUN cmake .. -DCMAKE_BUILD_TYPE=Release -DMOLD_USE_SYSTEM_TBB=ON -DMOLD_USE_MIMALLOC=OFF
RUN make -j16
WORKDIR /mold/build-wild
RUN cmake .. -DCMAKE_BUILD_TYPE=Release -DMOLD_USE_SYSTEM_TBB=ON -DMOLD_USE_MIMALLOC=OFF -DCMAKE_EXE_LINKER_FLAGS="-B /wild"
WORKDIR /wild
RUN cargo b --release

# Build wild linker late (after we run cmake for the 2nd time).
WORKDIR /mold/build-wild
RUN make -j16

RUN /mold/build/mold || true
RUN /mold/build-wild/mold || true
