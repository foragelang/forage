//! Service-layer barrel. Components and consumers import from here.

export { StudioServiceProvider, useStudioService } from "./context";
export {
    NotSupportedByService,
    StaleBaseError,
    type AlignmentUri,
    type DebugAction,
    type DeeplinkClonePayload,
    type DeviceStart,
    type ForkedFrom,
    type ListVersionsItem,
    type PackageFixture,
    type PackageListing,
    type PackageMetadata,
    type PackageQuery,
    type PackageSnapshot,
    type PackageVersion,
    type PollOutcome,
    type PublishError,
    type PublishOutcome,
    type PublishPayload,
    type PublishPreview,
    type PublishTypePayload,
    type ServiceCapabilities,
    type StudioService,
    type SyncOutcomeWire,
    type TypeFieldAlignment,
    type TypeMetadata,
    type TypeRef,
    type TypeVersion,
    type Unsubscribe,
} from "./StudioService";
export { TauriStudioService } from "./TauriStudioService";
