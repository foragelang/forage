import { useQuery } from "@tanstack/react-query";

import { Sidebar } from "@/components/Sidebar";
import { DeploymentView } from "@/components/Deployment/DeploymentView";
import { EditorView } from "@/components/EditorView";
import { BootSplash, Welcome } from "@/components/Welcome";
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar";
import { useStudioEffects } from "@/hooks/useStudioEffects";
import { api } from "@/lib/api";
import { currentWorkspaceKey } from "@/lib/queryKeys";
import { useStudio } from "@/lib/store";

/// The top-level branch: Studio either has a workspace open (the
/// existing editor/deployment shell) or it doesn't (Welcome). The
/// query is the single source of truth — backend `forage:workspace-*`
/// events invalidate it through `useStudioEffects` so the menu and
/// the switcher popover land here without any local state.
export function App() {
    // Mount the global event/keyboard wiring once at the top level. This
    // includes the workspace-lifecycle listeners, so it has to run
    // regardless of which branch is rendered below.
    useStudioEffects();
    const ws = useQuery({
        queryKey: currentWorkspaceKey(),
        queryFn: api.currentWorkspace,
    });
    if (ws.isPending) return <BootSplash />;
    if (ws.data === null || ws.data === undefined) return <Welcome />;
    return <StudioShell />;
}

function StudioShell() {
    const view = useStudio((s) => s.view);
    return (
        <SidebarProvider defaultOpen>
            <Sidebar />
            <SidebarInset className="min-h-0">
                {view === "editor" ? <EditorView /> : <DeploymentView />}
            </SidebarInset>
        </SidebarProvider>
    );
}
