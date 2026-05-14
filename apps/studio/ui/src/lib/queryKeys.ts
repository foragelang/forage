//! Centralized TanStack Query keys.
//!
//! Cache keys are structural — two queries with the same first element
//! and different shape afterwards are two separate buckets. Building
//! keys via these helpers keeps the shape consistent across panes that
//! all watch the same logical resource at different page sizes.

export const scheduledRunsKey = (runId: string, opts: { limit: number }) =>
    ["scheduledRuns", runId, opts] as const;
