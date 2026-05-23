//! `corivo-mcp` library — wire types shared between the standalone
//! `corivo-mcp` sidecar binary and any in-process consumer that needs
//! to speak the same JSON envelope (currently `apps/desktop/src-tauri`'s
//! `mcp_bridge`, which serves the UDS side of the same protocol).

pub mod proto;
