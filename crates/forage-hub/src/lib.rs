//! Forage hub client.
//!
//! The unit of distribution is a **package version** — one indivisible
//! atomic artifact carrying the recipe, decls, fixtures, snapshot,
//! `base_version`, and optional fork lineage. Versions are linear per
//! package; publish rebases by re-fetching `latest` first.
//!
//! This crate provides:
//!
//! - [`HubClient`] — REST surface (get / publish / fork / download /
//!   whoami) against `api.foragelang.com`.
//! - [`operations`] — high-level helpers that read/write the local
//!   workspace alongside the REST calls: `sync_from_hub`,
//!   `fork_from_hub`, `publish_from_workspace`, plus the
//!   `.forage-meta.json` sidecar that tracks each synced recipe's
//!   origin and `base_version`.
//! - [`AuthStore`] — file-backed JWT store.
//! - [`device`] — GitHub OAuth device-code flow.

pub mod auth_store;
pub mod client;
pub mod device;
pub mod error;
pub mod operations;
pub mod types;

pub use auth_store::{AuthStore, AuthTokens};
pub use client::HubClient;
pub use forage_core::workspace::{hub_cache_root, resolve_dep};
pub use device::{DevicePollResponse, DeviceStartResponse, poll_device, start_device};
pub use error::{HubError, HubResult};
pub use operations::{
    FetchedPackage, ForageMeta, META_SIDECAR_NAME, SyncOutcome, assemble_publish_request,
    core_snapshot_to_wire, fetch_to_cache, fork_from_hub, meta_path, publish_from_workspace,
    read_meta, sync_from_hub, write_meta,
};
pub use types::{
    ForkRequest, ForkedFrom, PackageFile, PackageFixture, PackageMetadata, PackageSnapshot,
    PackageVersion, PublishRequest, PublishResponse, VersionSpec,
};
