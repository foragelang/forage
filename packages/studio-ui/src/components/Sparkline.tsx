//! Tiny inline SVG sparkline. No chart library — the shape is one
//! polyline through normalized points, optionally overlaid with red
//! anomaly dots at named indices. Used by the run pane, history pane,
//! and deployment trends.

import { useMemo } from "react";

export type SparklineProps = {
    values: number[];
    width?: number;
    height?: number;
    /// Indices in `values` to mark as red anomaly dots (drift/fail).
    anomalies?: number[];
    className?: string;
};

export function Sparkline({
    values,
    width = 80,
    height = 20,
    anomalies,
    className,
}: SparklineProps) {
    const points = useMemo(() => normalize(values, width, height), [
        values,
        width,
        height,
    ]);
    if (points.length === 0) {
        return (
            <svg
                width={width}
                height={height}
                className={className}
                aria-hidden="true"
            />
        );
    }
    const path = points
        .map(([x, y], i) => (i === 0 ? `M${x},${y}` : `L${x},${y}`))
        .join(" ");
    return (
        <svg
            width={width}
            height={height}
            viewBox={`0 0 ${width} ${height}`}
            preserveAspectRatio="none"
            className={className}
            aria-hidden="true"
        >
            <path
                d={path}
                fill="none"
                stroke="currentColor"
                strokeWidth="1"
                strokeLinejoin="round"
                strokeLinecap="round"
            />
            {anomalies?.map((i) => {
                const p = points[i];
                if (!p) return null;
                return (
                    <circle
                        key={i}
                        cx={p[0]}
                        cy={p[1]}
                        r={2}
                        // Anomaly dots always render in the destructive
                        // token so they stand out against any line color.
                        fill="var(--destructive)"
                    />
                );
            })}
        </svg>
    );
}

/// Map `values` to `[width × height]` pixel space. Single-value series
/// renders as a horizontal mid-line so the user still sees the position.
function normalize(values: number[], width: number, height: number): [number, number][] {
    if (values.length === 0) return [];
    const min = Math.min(...values);
    const max = Math.max(...values);
    const span = max - min === 0 ? 1 : max - min;
    const pad = 1;
    return values.map((v, i) => {
        const x =
            values.length === 1
                ? width / 2
                : (i / (values.length - 1)) * (width - pad * 2) + pad;
        const y = height - pad - ((v - min) / span) * (height - pad * 2);
        return [x, y];
    });
}
