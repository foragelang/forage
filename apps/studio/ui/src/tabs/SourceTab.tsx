import Editor, { type Monaco } from "@monaco-editor/react";
import { useEffect, useRef } from "react";

import { FORAGE_LANG_ID, registerForageLanguage } from "../lib/monaco-forage";
import { useStudio } from "../lib/store";

export function SourceTab() {
    const { source, setSource, validation } = useStudio();
    const monacoRef = useRef<Monaco | null>(null);

    useEffect(() => {
        if (!monacoRef.current) return;
        const monaco = monacoRef.current;
        const issues = validation
            ? [
                  ...(validation.errors || []).map((m) => ({ message: m, severity: 8 })),
                  ...(validation.warnings || []).map((m) => ({ message: m, severity: 4 })),
              ]
            : [];
        const models = monaco.editor.getModels();
        for (const model of models) {
            monaco.editor.setModelMarkers(
                model,
                "forage",
                issues.map((i) => ({
                    severity: i.severity,
                    message: i.message,
                    startLineNumber: 1,
                    startColumn: 1,
                    endLineNumber: 1,
                    endColumn: 1,
                })),
            );
        }
    }, [validation]);

    return (
        <div className="flex-1 flex flex-col min-h-0">
            <div className="flex-1 min-h-0">
                <Editor
                    height="100%"
                    language={FORAGE_LANG_ID}
                    theme="vs-dark"
                    value={source}
                    onChange={(v) => setSource(v ?? "")}
                    beforeMount={(monaco) => {
                        monacoRef.current = monaco;
                        registerForageLanguage(monaco);
                    }}
                    options={{
                        fontSize: 13,
                        tabSize: 4,
                        minimap: { enabled: false },
                        wordWrap: "on",
                        scrollBeyondLastLine: false,
                        renderWhitespace: "selection",
                    }}
                />
            </div>
            {validation && (
                <div className="border-t border-zinc-800 px-4 py-2 max-h-32 overflow-y-auto text-xs">
                    {validation.ok && validation.warnings.length === 0 && (
                        <span className="text-emerald-400">✓ validates</span>
                    )}
                    {validation.errors.map((e, i) => (
                        <div key={`e${i}`} className="text-red-400">
                            <span className="text-red-300 font-medium">error:</span> {e}
                        </div>
                    ))}
                    {validation.warnings.map((w, i) => (
                        <div key={`w${i}`} className="text-amber-400">
                            <span className="text-amber-300 font-medium">warning:</span> {w}
                        </div>
                    ))}
                </div>
            )}
        </div>
    );
}
