FROM rust:slim-buster AS base
# C# https://learn.microsoft.com/en-us/dotnet/core/install/linux-debian
RUN apt update \
    && apt install -y wget \
    && wget https://packages.microsoft.com/config/debian/11/packages-microsoft-prod.deb -O packages-microsoft-prod.deb \
    && dpkg -i packages-microsoft-prod.deb \
    && rm packages-microsoft-prod.deb
# install deps
RUN apt update \
    && apt install -y \
        # core libraries
        libzstd-dev \
        # languages
        npm \
        python3 \
        openjdk-11-jdk \
        dotnet-sdk-6.0 \
        check valgrind \
        r-base r-cran-devtools \
    # https://github.com/testmycode/tmc-r-tester/
    && R -e 'devtools::install_github("testmycode/tmc-r-tester/tmcRtestrunner", build = FALSE)'
RUN cargo install cargo-chef --locked
WORKDIR /build

FROM base AS chef
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM base AS builder
COPY --from=chef /build/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --recipe-path recipe.json
# Build application
COPY . .
RUN cargo build
