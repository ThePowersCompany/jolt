FROM rust:1-bookworm AS builder

WORKDIR /workspace

COPY Cargo.toml Cargo.lock ./
COPY joltr-core ./joltr-core
COPY joltr-db ./joltr-db
COPY joltr-macros ./joltr-macros
COPY joltr-templates ./joltr-templates
COPY joltr-types ./joltr-types
COPY joltr-utils ./joltr-utils
COPY examples/basic ./examples/basic

RUN cargo build --release -p joltr-basic-example
RUN JOLTR_TYPES_OUT=/workspace/types.d.ts \
    /workspace/target/release/joltr-basic-example --generate-types

FROM debian:bookworm-slim AS runtime

WORKDIR /workspace

COPY --from=builder /workspace/target/release/joltr-basic-example /usr/local/bin/joltr-basic-example
COPY --from=builder /workspace/examples/basic/templates /workspace/examples/basic/templates
COPY --from=builder /workspace/examples/basic/public /workspace/examples/basic/public
COPY --from=builder /workspace/examples/basic/migrations /workspace/examples/basic/migrations
COPY --from=builder /workspace/types.d.ts /workspace/types.d.ts

EXPOSE 3000

CMD ["/usr/local/bin/joltr-basic-example"]
