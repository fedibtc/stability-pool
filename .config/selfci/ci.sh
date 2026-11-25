#!/usr/bin/env bash
set -eou pipefail

function job_lint() {
  selfci step start "cargo fmt"
  if ! nix build --accept-flake-config -L .#ci.cargoFmt ; then
    selfci step fail
  fi

  selfci step start "treefmt"
  if ! nix build --accept-flake-config -L .#treefmt ; then
    selfci step fail
  fi
}

function job_cargo() {
  # Submit clippy and tests together so Nix can share and schedule their
  # common CI-profile build graph.
  local system
  system=$(nix eval --raw --impure --expr builtins.currentSystem)

  selfci step start "nix cargo checks"
  if ! nix build --accept-flake-config -L --no-link --max-jobs 16 \
      ".#ci.${system}.clippy" \
      ".#ci.${system}.tests" ; then
    selfci step fail
  fi
}

case "$SELFCI_JOB_NAME" in
  main)
    selfci job start "lint"
    selfci job start "cargo"
    ;;
  cargo)
    job_cargo
    ;;
  lint)
    job_lint
    ;;
  *)
    echo "Unknown job: $SELFCI_JOB_NAME"
    exit 1
esac
