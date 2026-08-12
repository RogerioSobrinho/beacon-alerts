# Changelog

## Unreleased

## 0.1.0-rc3

- Split the runtime into `beacon-server` and `beacon-agent` binaries.
- Added one-time, expiring agent enrollment with server-generated credentials.
- Updated the container, Compose deployment, CI, systemd unit, and homelab pilot
  documentation for the split runtime.

## 0.1.0-rc2

- Created the Rust single-binary bootstrap.
- Added provisional event protocol v1.
- Added architecture, security, and operations documents.
- Added initial CLI subcommands: `server`, `agent`, `send`, and `replay`.
