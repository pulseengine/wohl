//! Wohl Matter Core — WASM component glue (SPIKE, SWARCH-WOHL-008).
//!
//! Proves the rs-matter protocol core compiles into a WebAssembly
//! component built by `rules_wasm_component` (`rust_wasm_component_bindgen`,
//! `wasi_version = "p3"`), with `rs-matter` supplied as a crate-universe
//! dependency (`@crates//:rs-matter`, features = ["rustcrypto"], no `os`
//! feature → no sockets/polling).
//!
//! The exported functions deliberately read a real rs-matter symbol
//! (`rs_matter::MATTER_PORT`) so the dependency is genuinely linked
//! into the component, not dead-code-eliminated. This is the smallest
//! increment that answers "can the Matter core be a WASM component?".
//!
//! Step 2 replaces this with the real seam: a `network` import
//! (rs-matter's `NetworkSend`/`NetworkReceive` mapped to WIT funcs),
//! `clock`/`random`/`persist` imports, and a publish/commission export
//! surface — composed with a host-import transport via `wac_compose`.

use wohl_matter_core_bindings::exports::pulseengine::wohl_matter_core::matter_core::Guest;

struct Component;

impl Guest for Component {
    // P3 async on the wasm32 target; plain sync off-target (mirrors the
    // monitor components' dual-cfg shape).
    #[cfg(target_arch = "wasm32")]
    async fn matter_port() -> u16 {
        rs_matter::MATTER_PORT
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn matter_port() -> u16 {
        rs_matter::MATTER_PORT
    }

    #[cfg(target_arch = "wasm32")]
    async fn is_commissioned() -> bool {
        false
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn is_commissioned() -> bool {
        false
    }
}

wohl_matter_core_bindings::export!(Component with_types_in wohl_matter_core_bindings);
