//! Library surface of the `syncthing` binary crate.
//!
//! The binary is the primary product; these modules are exposed so that
//! integration tests and benches can reach the REST API server and the
//! health predictor without shelling out to the executable.

/// REST API server startup and config store wiring.
pub mod api_server;
/// Predictive health checks over sync-engine event trends.
pub mod health_predictor;
