# Wraps the sibling relay checkout's verified plain crates as rust_library
# targets, so wohl's monitor components can depend on the REAL engines
# instead of inline copies (#84). Mirrors relay's own BUILD.bazel targets
# `relay-primitives-lib` and `relay-lc-engine` (relay#168 precedent) —
# keep in sync with relay/BUILD.bazel if those definitions move.
#
# Why not `bazel_dep` on the relay module: relay's MODULE.bazel registers
# Lean/Rocq/Verus toolchains and uses local_path_override (root-only), so
# consuming it as a module would drag those toolchains into every wohl
# build or break resolution. This repo rule only reads source files.

load("@rules_rust//rust:defs.bzl", "rust_library")

package(default_visibility = ["//visibility:public"])

rust_library(
    name = "relay-primitives-lib",
    srcs = glob(["crates/relay-primitives/plain/src/**/*.rs"]),
    edition = "2024",
    crate_name = "relay_primitives",
)

rust_library(
    name = "relay-lc-engine",
    srcs = glob(["crates/relay-lc/plain/src/**/*.rs"]),
    edition = "2024",
    crate_name = "relay_lc",
    deps = [":relay-primitives-lib"],
)

rust_library(
    name = "relay-to-engine",
    srcs = glob(["crates/relay-to/plain/src/**/*.rs"]),
    edition = "2024",
    crate_name = "relay_to",
)
