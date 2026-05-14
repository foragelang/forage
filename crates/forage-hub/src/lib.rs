//! Forage hub client.
//!
//! The unit of distribution is a **package** — a directory of `.forage`
//! files (recipes and shared declarations) plus a `forage.toml`
//! manifest. Single-file recipes ship as one-file packages. The wire
//! shape mirrors that layout: `fetch_package` returns every file in a
//! version's package; `publish_package` accepts the same.
//!
//! This crate provides:
//!
//! - [`HubClient`] — REST surface (list / get / publish / delete)
//!   against `api.foragelang.com`.
//! - [`fetch_package`] / [`publish_package`] — high-level helpers that
//!   read/write the on-disk cache at
//!   `~/Library/Forage/Cache/hub/<author>/<slug>/<version>/`.
//! - [`AuthStore`] — keychain-backed JWT store.
//! - [`device`] — GitHub OAuth device-code flow.

pub mod auth_store;
pub mod client;
pub mod device;
pub mod error;
pub mod types;

pub use auth_store::{AuthStore, AuthTokens};
pub use client::{
    FetchedPackage, HubClient, fetch_package, hub_cache_root, package_cache_dir, publish_package,
    resolve_dep,
};
pub use device::{DevicePollResponse, DeviceStartResponse, poll_device, start_device};
pub use error::{HubError, HubResult};
pub use types::{Package, PackageFile, PackageMeta};
