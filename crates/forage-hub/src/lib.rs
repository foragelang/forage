//! Forage hub client: list / get / publish / delete against api.foragelang.com,
//! recursive `import hub://...` resolution, on-disk auth store, and the
//! GitHub OAuth device-code flow.

pub mod auth_store;
pub mod client;
pub mod device;
pub mod error;

pub use auth_store::{AuthStore, AuthTokens};
pub use client::{HubClient, RecipeBlob, RecipeMeta};
pub use device::{DevicePollResponse, DeviceStartResponse, poll_device, start_device};
pub use error::{HubError, HubResult};
