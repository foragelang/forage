//! Right inspector. Three modes via segmented tabs at the top:
//! "This run" (live + last-known), "History" (per-session run rollup),
//! "Records" (rows from the latest scheduled run).
//!
//! The mode lives in the store (`inspectorMode`) so the choice
//! persists across file switches.

import {
    Tabs,
    TabsContent,
    TabsList,
    TabsTrigger,
} from "@/components/ui/tabs";

import { useStudio, type InspectorMode } from "@/lib/store";

import { HistoryPane } from "./HistoryPane";
import { RecordsPane } from "./RecordsPane";
import { RunPane } from "./RunPane";

export function Inspector({ width }: { width: number }) {
    const inspectorMode = useStudio((s) => s.inspectorMode);
    const setInspectorMode = useStudio((s) => s.setInspectorMode);
    return (
        <aside
            style={{ width }}
            className="shrink-0 min-h-0 flex flex-col"
        >
            <Tabs
                value={inspectorMode}
                onValueChange={(v) => setInspectorMode(v as InspectorMode)}
                className="flex-1 min-h-0 gap-0"
            >
                <div className="border-b px-3 shrink-0">
                    <TabsList variant="line" className="h-10">
                        <TabsTrigger value="run">This run</TabsTrigger>
                        <TabsTrigger value="history">History</TabsTrigger>
                        <TabsTrigger value="records">Records</TabsTrigger>
                    </TabsList>
                </div>
                <TabsContent
                    value="run"
                    className="flex-1 min-h-0 m-0 flex flex-col data-[state=inactive]:hidden"
                >
                    <RunPane />
                </TabsContent>
                <TabsContent
                    value="history"
                    className="flex-1 min-h-0 m-0 flex flex-col data-[state=inactive]:hidden"
                >
                    <HistoryPane />
                </TabsContent>
                <TabsContent
                    value="records"
                    className="flex-1 min-h-0 m-0 flex flex-col data-[state=inactive]:hidden"
                >
                    <RecordsPane />
                </TabsContent>
            </Tabs>
        </aside>
    );
}
