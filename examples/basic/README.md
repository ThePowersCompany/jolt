# JoltR Basic Example

Run the example locally over plain HTTP:

```sh
cargo run -p joltr-basic-example
```

The app listens on `http://localhost:3000` by default.

Build and run the example Docker image from the workspace root:

```sh
docker build -f examples/basic/Dockerfile -t joltr-basic-example .
docker run --rm -p 3000:3000 joltr-basic-example
```

The image starts without external services by default. `DATABASE_URL` is optional; when it is unset, database migrations are skipped.

Run the containerized TypeScript integration test from the workspace root:

```sh
examples/basic/integration/run.sh
```

The script builds the Docker image unless `JOLTR_BASIC_SKIP_DOCKER_BUILD=1` is set, starts the container on a random localhost port, waits for `GET /api/test/typed`, copies `/workspace/types.d.ts` into `target/joltr-basic-integration/types.d.ts`, type-checks the TypeScript test, and verifies the runtime response.

To reuse a prebuilt image:

```sh
docker build -t joltr-basic-example:local -f examples/basic/Dockerfile .
JOLTR_BASIC_IMAGE=joltr-basic-example:local JOLTR_BASIC_SKIP_DOCKER_BUILD=1 examples/basic/integration/run.sh
```

Generate the TypeScript declarations without starting the server:

```sh
JOLTR_TYPES_OUT=target/joltr-basic-example-types.d.ts cargo run -q -p joltr-basic-example -- --generate-types
```

The generated declarations include `TypedTestResponse`, used by the integration test.

The typed test endpoint is available at `http://127.0.0.1:3000/api/test/typed` during local HTTP startup and returns:

```json
{
  "contract_version": 1,
  "service": "joltr-basic-example",
  "ok": true,
  "features": ["endpoint-macro", "ts-export"]
}
```

The final local Rust-port verification path is:

```sh
cargo fmt --check
cargo check --workspace --all-targets
cargo test --workspace
docker build -t joltr-basic-example:local -f examples/basic/Dockerfile .
JOLTR_TYPES_OUT=target/joltr-basic-example-types.d.ts cargo run -q -p joltr-basic-example -- --generate-types
examples/basic/integration/run.sh
```

To enable TLS, set both certificate and private-key path variables before startup:

```sh
JOLTR_BASIC_TLS_CERT_CHAIN=./certs/localhost.crt \
JOLTR_BASIC_TLS_PRIVATE_KEY=./certs/localhost.key \
cargo run -p joltr-basic-example
```

When both variables are set, the server listens on `https://localhost:3000`. When neither is set, startup remains plain HTTP. Setting only one TLS variable is treated as a configuration error.
