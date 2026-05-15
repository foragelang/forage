//! Health badge with a leading colored dot. Wraps shadcn `Badge` so
//! the variant tokens stay consistent with the rest of the UI.
//!
//! `health` decides both the badge variant and the dot color; the
//! optional `children` overrides the default text label.

import { type ReactNode } from "react";

import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import type { Health } from "@/bindings/Health";

type HealthVariant = "success" | "warning" | "destructive" | "ghost";

const HEALTH_TO_VARIANT: Record<Health, HealthVariant> = {
    ok: "success",
    drift: "warning",
    fail: "destructive",
    paused: "ghost",
    unknown: "ghost",
};

const HEALTH_TO_DOT: Record<Health, string> = {
    ok: "bg-success",
    drift: "bg-warning",
    fail: "bg-destructive",
    paused: "bg-muted-foreground/50",
    unknown: "bg-muted-foreground/50",
};

const DEFAULT_LABEL: Record<Health, string> = {
    ok: "healthy",
    drift: "drift",
    fail: "failed",
    paused: "paused",
    unknown: "unknown",
};

export type StatusPillProps = {
    health: Health;
    children?: ReactNode;
    className?: string;
};

export function StatusPill({ health, children, className }: StatusPillProps) {
    return (
        <Badge variant={HEALTH_TO_VARIANT[health]} className={className}>
            <span className={cn("size-1.5 shrink-0 rounded-full", HEALTH_TO_DOT[health])} />
            {children ?? DEFAULT_LABEL[health]}
        </Badge>
    );
}
