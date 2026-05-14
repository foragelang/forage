//! Centralized TanStack Query keys.
//!
//! Cache keys are structural — two queries with the same first element
//! and different shape afterwards are two separate buckets. Building
//! keys via these helpers keeps the shape consistent across panes that
//! all watch the same logical resource at different page sizes.

export const scheduledRunsKey = (runId: string, opts: { limit: number }) =>
    ["scheduledRuns", runId, opts] as const;

/// The boot-blocking query: which workspace, if any, is currently open.
/// Invalidated by `forage:workspace-opened` and `forage:workspace-closed`
/// events from the backend so the App's top-level branch flips between
/// Welcome and StudioShell without a reload.
export const currentWorkspaceKey = () => ["currentWorkspace"] as const;

/// The Welcome view's recents list. Invalidated on workspace open so a
/// freshly-opened workspace floats to the top of the list before the
/// user returns to Welcome.
export const recentWorkspacesKey = () => ["recentWorkspaces"] as const;
