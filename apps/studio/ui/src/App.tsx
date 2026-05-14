import { Sidebar } from "@/components/Sidebar";
import { DeploymentView } from "@/components/Deployment/DeploymentView";
import { EditorView } from "@/components/EditorView";
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar";
import { useStudioEffects } from "@/hooks/useStudioEffects";
import { useStudio } from "@/lib/store";

export function App() {
    const view = useStudio((s) => s.view);
    useStudioEffects();
    return (
        <SidebarProvider defaultOpen>
            <Sidebar />
            <SidebarInset className="min-h-0">
                {view === "editor" ? <EditorView /> : <DeploymentView />}
            </SidebarInset>
        </SidebarProvider>
    );
}
