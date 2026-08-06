//! Provider registry and provider-neutral capability metadata.
//!
//! Data transfer remains implemented by each provider module, while the app
//! and CLI use this registry for identity, availability and supported connect
//! modes. Adding a provider must not require another hard-coded provider row.

use crate::sync::config::SyncProvider;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderAvailability {
    Available,
    Unavailable(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderCapabilities(u8);

impl ProviderCapabilities {
    pub const OPEN_REMOTE: Self = Self(1 << 0);
    pub const PUBLISH_LOCAL: Self = Self(1 << 1);
    pub const REAUTHENTICATE: Self = Self(1 << 2);
    pub const WATCH_REMOTE: Self = Self(1 << 3);

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, capability: Self) -> bool {
        self.0 & capability.0 == capability.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderDescriptor {
    pub provider: SyncProvider,
    pub display_name: &'static str,
    pub subtitle: &'static str,
    pub availability: ProviderAvailability,
    pub capabilities: ProviderCapabilities,
}

pub trait SyncProviderDescriptor: Send + Sync {
    fn descriptor(&self) -> &'static ProviderDescriptor;
}

#[derive(Debug)]
struct StaticProvider(&'static ProviderDescriptor);

impl SyncProviderDescriptor for StaticProvider {
    fn descriptor(&self) -> &'static ProviderDescriptor {
        self.0
    }
}

static SHAREPOINT: ProviderDescriptor = ProviderDescriptor {
    provider: SyncProvider::SharePoint,
    display_name: "SharePoint",
    subtitle: "Microsoft 365 · team document libraries",
    availability: ProviderAvailability::Available,
    capabilities: ProviderCapabilities::OPEN_REMOTE.union(ProviderCapabilities::REAUTHENTICATE),
};

#[cfg(target_os = "macos")]
static ICLOUD: ProviderDescriptor = ProviderDescriptor {
    provider: SyncProvider::ICloudDrive,
    display_name: "iCloud Drive",
    subtitle: "Apple iCloud · user-selected vaults",
    availability: ProviderAvailability::Available,
    capabilities: ProviderCapabilities::OPEN_REMOTE
        .union(ProviderCapabilities::PUBLISH_LOCAL)
        .union(ProviderCapabilities::WATCH_REMOTE),
};

#[cfg(not(target_os = "macos"))]
static ICLOUD: ProviderDescriptor = ProviderDescriptor {
    provider: SyncProvider::ICloudDrive,
    display_name: "iCloud Drive",
    subtitle: "Apple iCloud · macOS only",
    availability: ProviderAvailability::Unavailable("iCloud Drive requires macOS"),
    capabilities: ProviderCapabilities(0),
};

static SHAREPOINT_PROVIDER: StaticProvider = StaticProvider(&SHAREPOINT);
static ICLOUD_PROVIDER: StaticProvider = StaticProvider(&ICLOUD);

pub struct ProviderRegistry;

impl ProviderRegistry {
    pub fn all() -> [&'static dyn SyncProviderDescriptor; 2] {
        [&SHAREPOINT_PROVIDER, &ICLOUD_PROVIDER]
    }

    pub fn get(provider: SyncProvider) -> &'static dyn SyncProviderDescriptor {
        match provider {
            SyncProvider::SharePoint => &SHAREPOINT_PROVIDER,
            SyncProvider::ICloudDrive => &ICLOUD_PROVIDER,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_unique_provider_ids() {
        let providers = ProviderRegistry::all();
        assert_ne!(
            providers[0].descriptor().provider,
            providers[1].descriptor().provider
        );
    }

    #[test]
    fn sharepoint_and_icloud_advertise_their_connect_modes() {
        assert!(ProviderRegistry::get(SyncProvider::SharePoint)
            .descriptor()
            .capabilities
            .contains(ProviderCapabilities::OPEN_REMOTE));
        #[cfg(target_os = "macos")]
        assert!(ProviderRegistry::get(SyncProvider::ICloudDrive)
            .descriptor()
            .capabilities
            .contains(ProviderCapabilities::PUBLISH_LOCAL));
    }
}
