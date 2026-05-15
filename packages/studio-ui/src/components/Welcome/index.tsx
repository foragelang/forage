//! Welcome — the no-workspace state. Centered card with Open / New
//! actions and a list of recent workspaces.
//!
//! Design source: `plans/workspace-lifecycle-design/welcome.jsx` +
//! `.wa-*` rules in `styles.css`. The CSS variables `--wa-*` mirror the
//! design's oklch tokens; they live in `styles.css` so the Welcome
//! shell and the switcher's amber ring can share them.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Folder, Plus } from "lucide-react";

import type { RecentWorkspace } from "@/bindings/RecentWorkspace";
import { useStudioService } from "@/lib/services";
import { recentWorkspacesKey } from "@/lib/queryKeys";
import {
    newWorkspaceAction,
    openRecentWorkspaceAction,
    openWorkspaceAction,
} from "@/lib/studioActions";

/// The amber Forage mark used in the Welcome header. Three strokes —
/// the stem and two leaves. Copy of the inline SVG from
/// `welcome.jsx` (design source); kept verbatim per the design brief.
function ForageMark() {
    return (
        <svg
            viewBox="0 0 32 32"
            width="36"
            height="36"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinecap="round"
            strokeLinejoin="round"
        >
            <path d="M16 28 V18" />
            <path d="M16 18 C 10 18, 6 14, 5 7 C 11 7, 15 11, 16 18" />
            <path d="M16 18 C 22 18, 26 14, 27 7 C 21 7, 17 11, 16 18" />
        </svg>
    );
}

function WelcomeKbd({ children }: { children: React.ReactNode }) {
    return (
        <kbd className="welcome-kbd inline-flex h-[18px] min-w-[18px] items-center justify-center rounded border px-1 font-mono text-[10px]">
            {children}
        </kbd>
    );
}

function WelcomeAction({
    icon,
    title,
    subtitle,
    accelerator,
    onClick,
}: {
    icon: React.ReactNode;
    title: string;
    subtitle: React.ReactNode;
    accelerator: [string, string];
    onClick: () => void;
}) {
    return (
        <button
            type="button"
            onClick={onClick}
            className="welcome-action flex items-center gap-3 rounded-[10px] border px-[13px] py-[11px] text-left"
        >
            <span className="welcome-action-icon inline-flex h-8 w-8 items-center justify-center rounded-[7px]">
                {icon}
            </span>
            <span className="flex min-w-0 flex-1 flex-col gap-[1px]">
                <span className="welcome-action-title text-[13px] font-medium">
                    {title}
                </span>
                <span className="welcome-action-sub text-[11px]">{subtitle}</span>
            </span>
            <span className="inline-flex gap-[3px]">
                <WelcomeKbd>{accelerator[0]}</WelcomeKbd>
                <WelcomeKbd>{accelerator[1]}</WelcomeKbd>
            </span>
        </button>
    );
}

function RecentRow({
    entry,
    onClick,
}: {
    entry: RecentWorkspace;
    onClick: () => void;
}) {
    return (
        <button
            type="button"
            onClick={onClick}
            className="welcome-recent flex items-center gap-[10px] rounded-[6px] px-[10px] py-[7px] text-left"
        >
            <Folder className="size-[14px] shrink-0 welcome-recent-icon" />
            <span className="flex min-w-0 flex-1 flex-col gap-[1px]">
                <span className="welcome-recent-name truncate text-[12px]">
                    {entry.name}
                </span>
                <span className="welcome-recent-path mono truncate text-[10px]">
                    {entry.path}
                </span>
            </span>
            <span className="welcome-recent-meta mono shrink-0 text-[10px]">
                {formatLastOpened(entry.opened_at)}
            </span>
        </button>
    );
}

/// Relative time label for the recents list. The exact cutoffs match
/// the design copy ("just now", "5m ago", "2d ago", …). Uses the local
/// system clock; no time zone awareness because the recents file
/// stores absolute UTC ms.
function formatLastOpened(opened_at: number): string {
    const diff = Date.now() - opened_at;
    if (diff < 60_000) return "just now";
    const minutes = Math.floor(diff / 60_000);
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.floor(hours / 24);
    if (days < 7) return `${days}d ago`;
    const weeks = Math.floor(days / 7);
    if (weeks < 5) return `${weeks}w ago`;
    const months = Math.floor(days / 30);
    if (months < 12) return `${months}mo ago`;
    const years = Math.floor(days / 365);
    return `${years}y ago`;
}

export function Welcome() {
    const qc = useQueryClient();
    const service = useStudioService();
    const version = useQuery({
        queryKey: ["studio-version"],
        queryFn: () => service.version(),
    });
    const recents = useQuery({
        queryKey: recentWorkspacesKey(),
        queryFn: () => service.listRecentWorkspaces(),
    });

    const recentEntries = recents.data ?? [];

    return (
        <div className="welcome grid h-full min-h-0 grid-rows-[1fr_auto]">
            <div className="welcome-inner mx-auto w-full max-w-[460px] px-9 pt-14 pb-8">
                <div className="welcome-head mb-[26px] text-center">
                    <div className="welcome-mark mb-3 inline-flex">
                        <ForageMark />
                    </div>
                    <h1 className="welcome-title m-0 mb-1 text-[22px] font-medium tracking-[-0.02em]">
                        Forage Studio
                    </h1>
                    <p className="welcome-tag m-0 text-[12px]">
                        Author recipes. Manage runs. Watch data over time.
                    </p>
                </div>

                <div className="welcome-actions mb-[22px] flex flex-col gap-[6px]">
                    <WelcomeAction
                        icon={<Folder className="size-[14px]" />}
                        title="Open workspace"
                        subtitle={
                            <>
                                Point at a folder that has a{" "}
                                <span className="mono">forage.toml</span>
                            </>
                        }
                        accelerator={["⌘", "O"]}
                        onClick={() => void openWorkspaceAction(qc)}
                    />
                    <WelcomeAction
                        icon={<Plus className="size-[14px]" />}
                        title="New workspace"
                        subtitle={
                            <>
                                Scaffold a fresh folder with{" "}
                                <span className="mono">forage.toml</span>
                            </>
                        }
                        accelerator={["⌘", "N"]}
                        onClick={() => void newWorkspaceAction(qc)}
                    />
                </div>

                {recentEntries.length > 0 && (
                    <>
                        <div className="welcome-section mb-2 text-[9px] font-semibold uppercase tracking-[0.1em]">
                            Recent workspaces
                        </div>
                        <div className="welcome-recents flex flex-col gap-[2px]">
                            {recentEntries.map((entry) => (
                                <RecentRow
                                    key={entry.path}
                                    entry={entry}
                                    onClick={() =>
                                        void openRecentWorkspaceAction(qc, entry.path)
                                    }
                                />
                            ))}
                        </div>
                    </>
                )}
            </div>
            <div className="welcome-foot mono px-3 py-3 text-center text-[10px]">
                forage &middot; v{version.data ?? "?"} &middot; daemon offline
            </div>
        </div>
    );
}

/// A boot placeholder that holds the screen while the
/// `currentWorkspace` query is still pending. Same mark and color band
/// as Welcome so the eventual transition doesn't jolt — the mark stays
/// put, the actions fade in.
export function BootSplash() {
    return (
        <div className="welcome grid h-full min-h-0 place-items-center">
            <div className="welcome-mark inline-flex">
                <ForageMark />
            </div>
        </div>
    );
}
