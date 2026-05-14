//! One record-type's count-over-time trend.
//!
//! Used in both the Inspector's History pane (compact, three rows) and
//! the Deployment view's body (wide, 60-run sparkline). Extracted from
//! the design's `TrendCard` JSX so both call sites share the rules for
//! tone, delta sign, and anomaly markers.

import { Card } from "@/components/ui/card";
import { Sparkline } from "@/components/Sparkline";
import { cn } from "@/lib/utils";

export type TrendCardProps = {
    typeName: string;
    series: number[];
    /// Last observed value; passed in (not derived) because callers
    /// sometimes have a live count that's fresher than the series tail.
    lastValue: number;
    /// Delta vs the previous run. Caller decides what "previous" means
    /// (last good run, last run, baseline) — we just render the sign.
    delta?: number;
    /// Indices in `series` to draw as red anomaly dots (drift/fail).
    anomalies?: number[];
    /// Visual scale. The Inspector History wants compact; Deployment
    /// trends want wide.
    size?: "compact" | "wide";
    className?: string;
};

export function TrendCard({
    typeName,
    series,
    lastValue,
    delta,
    anomalies,
    size = "compact",
    className,
}: TrendCardProps) {
    const wide = size === "wide";
    const width = wide ? 290 : 170;
    const height = wide ? 48 : 28;
    // Tone the line by health: red if last value is zero, amber if the
    // last value fell ≥5% below the series median, else green.
    const lineColor = toneClass(series, lastValue);
    return (
        <Card size="sm" className={cn("gap-2 p-3 ring-0", className)}>
            <div className="flex items-baseline justify-between gap-2">
                <span className="font-mono text-xs text-foreground">{typeName}</span>
                <div className="flex items-baseline gap-2 font-mono tabular-nums">
                    <span className="text-sm">{lastValue.toLocaleString()}</span>
                    {delta !== undefined && <DeltaBadge delta={delta} />}
                </div>
            </div>
            <Sparkline
                values={series}
                width={width}
                height={height}
                anomalies={anomalies}
                className={lineColor}
            />
        </Card>
    );
}

function DeltaBadge({ delta }: { delta: number }) {
    if (delta === 0) {
        return <span className="text-muted-foreground text-xs">±0</span>;
    }
    const positive = delta > 0;
    const tone = positive ? "text-success" : "text-destructive";
    const sign = positive ? "+" : "";
    return (
        <span className={cn("text-xs", tone)}>
            {sign}
            {delta.toLocaleString()}
        </span>
    );
}

function toneClass(series: number[], last: number): string {
    if (series.length === 0 || last === 0) return "text-destructive";
    const sorted = [...series].sort((a, b) => a - b);
    const median = sorted[Math.floor(sorted.length / 2)] ?? last;
    if (median > 0 && last < median * 0.95) return "text-warning";
    return "text-success";
}
