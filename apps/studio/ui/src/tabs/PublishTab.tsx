import { useMutation, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import {
    CheckCircle2,
    ExternalLink,
    Eye,
    LogIn,
    Loader2,
    LogOut,
    Send,
    XCircle,
} from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
    Card,
    CardContent,
    CardDescription,
    CardHeader,
    CardTitle,
} from "@/components/ui/card";
import {
    Dialog,
    DialogContent,
    DialogDescription,
    DialogFooter,
    DialogHeader,
    DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { api } from "@/lib/api";
import { useStudio } from "@/lib/store";

export function PublishTab() {
    const { activeSlug } = useStudio();
    const [hubUrl, setHubUrl] = useState("https://api.foragelang.com");
    const [showSignIn, setShowSignIn] = useState(false);

    const whoami = useQuery({
        queryKey: ["whoami", hubUrl],
        queryFn: () => api.authWhoami(hubUrl),
        staleTime: 30_000,
    });

    const publish = useMutation({
        mutationFn: ({ dryRun }: { dryRun: boolean }) =>
            api.publishRecipe(activeSlug!, hubUrl, dryRun),
    });

    const signedIn = !!whoami.data;
    const canPublish = !!activeSlug && signedIn && !publish.isPending;

    return (
        <ScrollArea className="flex-1">
            <div className="p-6 max-w-2xl space-y-6">
                <div>
                    <h2 className="text-base font-heading font-medium mb-1">Publish</h2>
                    <p className="text-sm text-muted-foreground">
                        Push this recipe to a Forage Hub so others can install it.
                    </p>
                </div>

                <Card size="sm">
                    <CardHeader>
                        <CardTitle>Hub</CardTitle>
                        <CardDescription>
                            Target Hub and authentication state.
                        </CardDescription>
                    </CardHeader>
                    <CardContent className="space-y-4">
                        <div className="space-y-1.5">
                            <Label htmlFor="hub-url">Hub URL</Label>
                            <Input
                                id="hub-url"
                                value={hubUrl}
                                onChange={(e) => setHubUrl(e.target.value)}
                                className="font-mono"
                                spellCheck={false}
                            />
                        </div>
                        <div className="space-y-1.5">
                            <Label>Signed in as</Label>
                            {whoami.isLoading ? (
                                <Badge variant="secondary" className="gap-1.5">
                                    <Loader2 className="animate-spin" />
                                    Checking…
                                </Badge>
                            ) : signedIn ? (
                                <div className="flex items-center gap-2">
                                    <Badge variant="success" className="gap-1">
                                        <CheckCircle2 />
                                        {whoami.data}
                                    </Badge>
                                    <Button
                                        size="xs"
                                        variant="ghost"
                                        onClick={async () => {
                                            await api.authLogout(hubUrl);
                                            whoami.refetch();
                                        }}
                                    >
                                        <LogOut />
                                        Sign out
                                    </Button>
                                </div>
                            ) : (
                                <Button onClick={() => setShowSignIn(true)} size="sm">
                                    <LogIn />
                                    Sign in with GitHub
                                </Button>
                            )}
                        </div>
                        <div className="space-y-1.5">
                            <Label>Slug</Label>
                            <div className="font-mono text-sm select-text">
                                {activeSlug ?? (
                                    <span className="text-muted-foreground">
                                        (no recipe selected)
                                    </span>
                                )}
                            </div>
                        </div>
                    </CardContent>
                </Card>

                <div className="flex items-center gap-2">
                    <Button
                        variant="outline"
                        disabled={!activeSlug || publish.isPending}
                        onClick={() => publish.mutate({ dryRun: true })}
                    >
                        <Eye />
                        Preview (dry-run)
                    </Button>
                    <Button
                        disabled={!canPublish}
                        onClick={() => publish.mutate({ dryRun: false })}
                    >
                        {publish.isPending ? (
                            <Loader2 className="animate-spin" />
                        ) : (
                            <Send />
                        )}
                        Publish
                    </Button>
                </div>

                {publish.data && (
                    <Card size="sm">
                        <CardHeader>
                            <CardTitle className="text-sm">
                                {publish.data.ok ? "Result" : "Error"}
                            </CardTitle>
                        </CardHeader>
                        <CardContent>
                            <pre className="text-xs font-mono whitespace-pre-wrap select-text">
                                {publish.data.error ||
                                    JSON.stringify(publish.data, null, 2)}
                            </pre>
                        </CardContent>
                    </Card>
                )}
                {publish.error && (
                    <Alert variant="destructive">
                        <XCircle />
                        <AlertTitle>Publish failed</AlertTitle>
                        <AlertDescription className="font-mono text-xs select-text">
                            {String(publish.error)}
                        </AlertDescription>
                    </Alert>
                )}

                <SignInDialog
                    open={showSignIn}
                    onOpenChange={setShowSignIn}
                    hubUrl={hubUrl}
                    onSuccess={() => {
                        setShowSignIn(false);
                        whoami.refetch();
                    }}
                />
            </div>
        </ScrollArea>
    );
}

function SignInDialog(props: {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    hubUrl: string;
    onSuccess: () => void;
}) {
    const [stage, setStage] = useState<
        "idle" | "started" | "polling" | "ok" | "error"
    >("idle");
    const [code, setCode] = useState("");
    const [url, setUrl] = useState("");
    const [err, setErr] = useState("");

    const reset = () => {
        setStage("idle");
        setCode("");
        setUrl("");
        setErr("");
    };

    const begin = async () => {
        try {
            setStage("started");
            const s = await api.authStartDeviceFlow(props.hubUrl);
            setCode(s.user_code);
            setUrl(s.verification_url);
            setStage("polling");
            const start = Date.now();
            const deadline = start + s.expires_in * 1000;
            while (Date.now() < deadline) {
                await new Promise((r) => setTimeout(r, s.interval * 1000));
                const p = await api.authPollDevice(props.hubUrl, s.device_code);
                if (p.status === "ok") {
                    setStage("ok");
                    setTimeout(props.onSuccess, 800);
                    return;
                }
                if (p.status === "expired") {
                    setErr("Device code expired. Try again.");
                    setStage("error");
                    return;
                }
            }
            setErr("Timed out waiting for the browser confirmation.");
            setStage("error");
        } catch (e) {
            setErr(String(e));
            setStage("error");
        }
    };

    return (
        <Dialog
            open={props.open}
            onOpenChange={(o) => {
                props.onOpenChange(o);
                if (!o) reset();
            }}
        >
            <DialogContent className="sm:max-w-[480px]">
                <DialogHeader>
                    <DialogTitle>Sign in to Forage Hub</DialogTitle>
                    <DialogDescription className="font-mono text-xs select-text">
                        {props.hubUrl}
                    </DialogDescription>
                </DialogHeader>

                {stage === "idle" && (
                    <>
                        <p className="text-sm text-muted-foreground">
                            Sign in to publish recipes under your GitHub account.
                        </p>
                        <DialogFooter>
                            <Button
                                variant="outline"
                                onClick={() => props.onOpenChange(false)}
                            >
                                Cancel
                            </Button>
                            <Button onClick={begin}>
                                <LogIn />
                                Start device-code flow
                            </Button>
                        </DialogFooter>
                    </>
                )}

                {(stage === "started" || stage === "polling") && (
                    <div className="space-y-4">
                        <ol className="text-sm space-y-3 list-decimal pl-5 text-muted-foreground">
                            <li>
                                Open{" "}
                                <a
                                    href={url}
                                    target="_blank"
                                    rel="noreferrer"
                                    className="inline-flex items-center gap-1 text-foreground underline underline-offset-2 hover:text-foreground/80"
                                >
                                    {url || "the verification URL"}
                                    <ExternalLink className="size-3" />
                                </a>
                            </li>
                            <li>Enter this code:</li>
                        </ol>
                        <div className="text-3xl font-mono font-medium tracking-[0.25em] text-center py-4 rounded-lg border bg-muted select-all">
                            {code || "…"}
                        </div>
                        <div className="flex items-center justify-center gap-2 text-xs text-muted-foreground">
                            <Loader2 className="size-3 animate-spin" />
                            Polling…
                        </div>
                    </div>
                )}

                {stage === "ok" && (
                    <Alert variant="success">
                        <CheckCircle2 />
                        <AlertTitle>Signed in</AlertTitle>
                    </Alert>
                )}

                {stage === "error" && (
                    <>
                        <Alert variant="destructive">
                            <XCircle />
                            <AlertTitle>Sign-in failed</AlertTitle>
                            <AlertDescription className="select-text">{err}</AlertDescription>
                        </Alert>
                        <DialogFooter>
                            <Button variant="outline" onClick={reset}>
                                Try again
                            </Button>
                        </DialogFooter>
                    </>
                )}
            </DialogContent>
        </Dialog>
    );
}
