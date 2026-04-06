//! Management API clients for the xAI Collections Management API.
//!
//! The Management API uses a separate base URL (`https://management-api.x.ai`)
//! and a separate authentication key from the inference API. This module
//! provides `ManagementClient` as the entry point, which wraps `HttpClient`
//! with the management-specific configuration.

pub mod client;
pub mod collections;
pub mod documents;
