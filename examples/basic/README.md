# JoltR Basic Example

Run the example locally over plain HTTP:

```sh
cargo run -p joltr-basic-example
```

The app listens on `http://localhost:3000` by default.

To enable TLS, set both certificate and private-key path variables before startup:

```sh
JOLTR_BASIC_TLS_CERT_CHAIN=./certs/localhost.crt \
JOLTR_BASIC_TLS_PRIVATE_KEY=./certs/localhost.key \
cargo run -p joltr-basic-example
```

When both variables are set, the server listens on `https://localhost:3000`. When neither is set, startup remains plain HTTP. Setting only one TLS variable is treated as a configuration error.
