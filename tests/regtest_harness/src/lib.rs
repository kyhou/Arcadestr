//! Container crate for the explicitly invoked regtest drivers in `examples/`.
//!
//! The drivers embed the adp-server reference implementation (router, storage,
//! settlement clock and master cipher) to inject ADP deadlines and inspect
//! server-side refund state against real Lightning nodes. They are therefore
//! kept out of Arcadestr's ordinary Cargo workspace. See `README.md`.
