FROM rust:slim-buster

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

COPY . /build
WORKDIR /build

RUN cargo check --tests --all-features
