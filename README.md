# devdeck

A native, keyboard- and mouse-friendly cockpit for developers: see all your apps and
services, pick an env, run their commands, watch output, and inspect or deploy
services on a Kubernetes cluster, without typing command lines.

Status: early development. See [docs/ROADMAP.md](docs/ROADMAP.md).

## Build

```sh
cargo build --release
./target/release/devdeck
```

Linux build dependencies (Debian/Ubuntu names) are listed in
[docs/BUILDING.md](docs/BUILDING.md).

## Docs

- [Requirements](docs/REQUIREMENTS.md)
- [Design](docs/DESIGN.md)
- [Workflow](docs/WORKFLOW.md)
