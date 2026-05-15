//! Collapsible HTML / XML tree viewer for the Response viewer's Tree
//! tab. Parses the raw body with the browser's `DOMParser` once on
//! mount; renders each element as `<tag attr="…">` with a collapse
//! triangle and indented children. Text nodes inline; comments + PIs
//! render dimmed.
//!
//! XML walks from `documentElement` so multi-root XML (which the
//! spec forbids but real-world feeds sometimes ship) lands a parse
//! error rather than silently dropping nodes.

import { useMemo, useState } from "react";
import { ChevronRight } from "lucide-react";

import { cn } from "@/lib/utils";

export function DomTree({
    source,
    mime,
}: {
    source: string;
    /// `text/html` for HTML mode, `application/xml` (or
    /// `text/xml`) for XML mode. Drives the `DOMParser` mime
    /// argument so each format goes through its own parser branch.
    mime: "text/html" | "application/xml";
}) {
    const root = useMemo(() => {
        const parser = new DOMParser();
        try {
            const doc = parser.parseFromString(source, mime);
            const errors = doc.getElementsByTagName("parsererror");
            if (errors.length > 0) {
                return { kind: "error" as const, message: errors[0]?.textContent ?? "parse error" };
            }
            if (mime === "text/html") {
                return { kind: "ok" as const, node: doc.documentElement };
            }
            return { kind: "ok" as const, node: doc.documentElement };
        } catch (e) {
            return { kind: "error" as const, message: String(e) };
        }
    }, [source, mime]);

    if (root.kind === "error") {
        return (
            <div className="p-3 text-xs text-destructive">
                Failed to parse: {root.message}
            </div>
        );
    }
    return (
        <div className="flex-1 overflow-y-auto p-2 font-mono text-xs select-text">
            <NodeView node={root.node} depth={0} />
        </div>
    );
}

function NodeView({ node, depth }: { node: Node; depth: number }) {
    const [open, setOpen] = useState<boolean>(depth < 2);

    if (node.nodeType === Node.TEXT_NODE) {
        const t = node.textContent?.trim() ?? "";
        if (t.length === 0) return null;
        return <span className="text-success">{t}</span>;
    }
    if (node.nodeType === Node.COMMENT_NODE) {
        return (
            <div className="text-muted-foreground italic">
                &lt;!-- {node.textContent} --&gt;
            </div>
        );
    }
    if (node.nodeType === Node.PROCESSING_INSTRUCTION_NODE) {
        const pi = node as ProcessingInstruction;
        return (
            <div className="text-muted-foreground italic">
                &lt;?{pi.target} {pi.data}?&gt;
            </div>
        );
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return null;

    const el = node as Element;
    const attrs: { name: string; value: string }[] = [];
    for (let i = 0; i < el.attributes.length; i += 1) {
        const a = el.attributes.item(i);
        if (a) attrs.push({ name: a.name, value: a.value });
    }
    const children: Node[] = [];
    for (let i = 0; i < el.childNodes.length; i += 1) {
        const c = el.childNodes.item(i);
        if (c) children.push(c);
    }
    const hasChildren = children.length > 0;

    return (
        <div className="py-0.5">
            <button
                type="button"
                onClick={() => setOpen((o) => !o)}
                className="flex items-baseline gap-1 w-full text-left"
                disabled={!hasChildren}
            >
                <ChevronRight
                    className={cn(
                        "size-3 transition-transform shrink-0",
                        open && hasChildren && "rotate-90",
                        !hasChildren && "opacity-0",
                    )}
                />
                <span className="text-foreground">
                    &lt;<span className="text-warning">{el.tagName.toLowerCase()}</span>
                    {attrs.map((a, i) => (
                        <span key={i}>
                            {" "}
                            <span className="text-muted-foreground">{a.name}</span>
                            =<span className="text-success">"{a.value}"</span>
                        </span>
                    ))}
                    {hasChildren ? ">" : " />"}
                </span>
            </button>
            {open && hasChildren && (
                <div className="ml-4 border-l border-border pl-2">
                    {children.map((c, i) => (
                        <NodeView key={i} node={c} depth={depth + 1} />
                    ))}
                </div>
            )}
            {open && hasChildren && (
                <div className="text-foreground">
                    &lt;/<span className="text-warning">{el.tagName.toLowerCase()}</span>&gt;
                </div>
            )}
        </div>
    );
}
