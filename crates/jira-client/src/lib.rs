//! Client Jira Cloud (REST API v3 + Jira Software Agile 1.0) pour jira-rust.
//!
//! Le trait [`JiraApi`] est le contrat avec la TUI. Deux implémentations :
//! - [`JiraHttp`] : client HTTP réel (Basic auth e-mail + token API) ;
//! - `mock::MockJira` (feature `mock`) : données fictives en mémoire.

pub mod adf;
mod api;
mod error;
mod guard;
mod http;
pub mod model;
mod secret;
mod timeutil;

#[cfg(feature = "mock")]
pub mod mock;

pub use api::JiraApi;
pub use error::{Error, Result};
pub use guard::OFFLINE_ENV;
pub use http::{JiraHttp, RetryPolicy};
pub use secret::SecretString;
