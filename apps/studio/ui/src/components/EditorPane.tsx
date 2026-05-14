//! Phase 4 stub of the editor pane. Re-exports the existing SourceTab
//! so the editor + breakpoints + validation continue to work under
//! the new shell. Phase 5 renames it (`tabs/SourceTab.tsx` →
//! `components/EditorPane.tsx`) and rebuilds the inline step-stats
//! widgets on top.

import { SourceTab } from "@/tabs/SourceTab";

export const EditorPane = SourceTab;
