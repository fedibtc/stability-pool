# Stability pool

The stability pool is a Fedimint module that lets users hold a stable fiat value
of their bitcoin, and lets groups hold bitcoin together under a k-of-n policy
(multispend). Seekers lock in a fiat value for a fee, and providers take the
other side. [docs/design.md](docs/design.md) explains how the auction, the
withdrawals and the cycle turnover work.

This repository holds the current module, module kind
`multi_sig_stability_pool`. The legacy `stability_pool` module stays in the
fedi repository.

## Crates

| Crate | Path | Used by |
|---|---|---|
| `fedi-stability-pool-common` | [`crates/common`](crates/common) | client and server: types, config and encoding |
| `fedi-stability-pool-client` | [`crates/client`](crates/client) | applications: the Fedi app, and the Fedimint SDK (planned) |
| `fedi-stability-pool-server` | [`crates/server`](crates/server) | guardians: Fedi's fedimintd |

## Building and testing

```sh
cargo test --workspace
```

The end-to-end tests that run a federation with this module still live in the
fedi repository (`crates/modules/stability-pool/tests`), because they need
Fedi's fedimintd.

## License

[AGPL-3.0](LICENSE).
