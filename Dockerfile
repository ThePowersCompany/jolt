FROM ghcr.io/thepowerscompany/zig:0.15.2 AS build

# Prevent interactive prompting
ENV DEBIAN_FRONTEND="noninteractive"

# Install system-wide build dependencies
# https://docs.docker.com/reference/dockerfile/#example-cache-apt-packages
RUN rm -f /etc/apt/apt.conf.d/docker-clean; echo 'Binary::apt::APT::Keep-Downloaded-Packages "true";' > /etc/apt/apt.conf.d/keep-cache
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
	--mount=type=cache,target=/var/lib/apt,sharing=locked \
	apt-get -y update && \
	apt-get -y dist-upgrade && \
	# TODO: Do we need these?
	apt-get -y install --no-install-recommends ca-certificates libssl-dev xz-utils && \
	apt-get -y autoremove

WORKDIR /project

COPY . .

WORKDIR /project/tests/server

RUN --mount=type=cache,target=/.zig-cache,sharing=locked \
	--mount=type=cache,target=/.global-zig-cache,sharing=locked \
	zig build --summary all -Doptimize=ReleaseSafe -Dcpu=x86_64_v3 -Dllvm=true --cache-dir /.zig-cache --global-cache-dir /.global-zig-cache

FROM gcr.io/distroless/base-debian13 AS server

COPY --from=build /project/tests/server/zig-out/bin/server /server
CMD ["/server"]

