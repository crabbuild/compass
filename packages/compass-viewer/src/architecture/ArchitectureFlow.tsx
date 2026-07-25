import { useMemo, useState } from "react";
import { ArrowRightIcon, BoxIcon, FileCodeIcon, NetworkIcon } from "lucide-react";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { ScrollArea } from "../components/ui/scroll-area";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../components/ui/tabs";
import type { CallflowViewModel } from "../contracts/callflow";

export type ArchitectureHost = {
  openSource(file: string): void;
};

export function ArchitectureFlow({
  model,
  host
}: {
  model: CallflowViewModel;
  host: ArchitectureHost;
}) {
  const first = model.sections.find((section) => section.id !== "overview")?.id ?? "overview";
  const [sectionId, setSectionId] = useState(first);
  const section = model.sections.find((candidate) => candidate.id === sectionId);
  const nodeNames = useMemo(
    () => new Map(model.sections.flatMap((item) => item.nodes.map((node) => [node.id, node.label]))),
    [model.sections]
  );
  return (
    <div className="architecture-shell">
      <aside className="architecture-nav">
        <header className="p-4">
          <span className="font-mono text-xs uppercase tracking-wider text-muted-foreground">
            Architecture flow
          </span>
          <h1 className="mt-1 text-base font-semibold">{model.title}</h1>
        </header>
        <ScrollArea className="min-h-0 flex-1">
          <nav className="flex flex-col gap-1 p-2" aria-label="Architecture sections">
            {model.sections.filter((item) => item.id !== "overview").map((item) => (
              <Button
                key={item.id}
                variant={item.id === sectionId ? "secondary" : "ghost"}
                className="h-auto justify-start py-2 text-left"
                onClick={() => setSectionId(item.id)}
              >
                <BoxIcon />
                <span className="min-w-0">
                  <span className="block truncate">{item.name}</span>
                  <span className="block text-xs text-muted-foreground">
                    {item.nodes.length} symbols · {item.edges.length} internal calls
                  </span>
                </span>
              </Button>
            ))}
          </nav>
        </ScrollArea>
        <footer className="grid grid-cols-3 gap-1 border-t p-3 text-center text-xs text-muted-foreground">
          <span><strong className="block text-foreground">{model.statistics.nodes}</strong>nodes</span>
          <span><strong className="block text-foreground">{model.statistics.edges}</strong>edges</span>
          <span><strong className="block text-foreground">{model.statistics.communities}</strong>groups</span>
        </footer>
      </aside>

      <main className="min-w-0 overflow-auto p-5">
        <section aria-labelledby="system-flow-heading">
          <div className="mb-3 flex items-center justify-between">
            <div>
              <h2 id="system-flow-heading" className="text-lg font-semibold">System call flow</h2>
              <p className="text-sm text-muted-foreground">
                Cross-subsystem relationships derived from the current Compass graph.
              </p>
            </div>
            <Badge variant="outline"><NetworkIcon /> {model.overviewLinks.length} flows</Badge>
          </div>
          <div className="architecture-flow-grid">
            {model.overviewLinks.slice(0, 24).map((link) => (
              <div
                key={`${link.sourceSection}:${link.targetSection}`}
                className="flex items-center gap-2 rounded-md border bg-card px-3 py-2 text-sm text-card-foreground"
              >
                <span className="truncate">{sectionName(model, link.sourceSection)}</span>
                <ArrowRightIcon className="shrink-0 text-muted-foreground" />
                <span className="truncate">{sectionName(model, link.targetSection)}</span>
                <Badge variant="secondary" className="ml-auto">{link.calls}</Badge>
              </div>
            ))}
          </div>
        </section>

        {section && (
          <section className="mt-6" aria-labelledby="section-heading">
            <h2 id="section-heading" className="text-lg font-semibold">{section.name}</h2>
            <Tabs defaultValue="symbols" className="mt-3">
              <TabsList>
                <TabsTrigger value="symbols">Symbols</TabsTrigger>
                <TabsTrigger value="calls">Call table</TabsTrigger>
              </TabsList>
              <TabsContent value="symbols" className="architecture-symbol-grid">
                {section.nodes.map((node) => (
                  <article key={node.id} className="rounded-md border bg-card p-3 text-card-foreground">
                    <div className="flex items-start gap-2">
                      <FileCodeIcon className="mt-0.5 shrink-0 text-muted-foreground" />
                      <div className="min-w-0">
                        <h3 className="truncate text-sm font-medium">{node.label}</h3>
                        <p className="truncate font-mono text-xs text-muted-foreground">
                          {node.kind || "symbol"}
                        </p>
                      </div>
                    </div>
                    {node.sourceFile && (
                      <Button
                        size="xs"
                        variant="ghost"
                        className="mt-2 max-w-full justify-start"
                        onClick={() => host.openSource(node.sourceFile!)}
                      >
                        <span className="truncate">{node.sourceFile}</span>
                      </Button>
                    )}
                  </article>
                ))}
              </TabsContent>
              <TabsContent value="calls">
                <div className="overflow-auto rounded-md border">
                  <table className="w-full text-left text-sm">
                    <thead className="bg-muted text-muted-foreground">
                      <tr><th className="p-2">Caller</th><th className="p-2">Relation</th><th className="p-2">Callee</th><th className="p-2">Evidence</th></tr>
                    </thead>
                    <tbody>
                      {section.edges.map((edge, index) => (
                        <tr key={`${edge.source}:${edge.target}:${index}`} className="border-t">
                          <td className="p-2">{nodeNames.get(edge.source) ?? edge.source}</td>
                          <td className="p-2">{edge.relation}</td>
                          <td className="p-2">{nodeNames.get(edge.target) ?? edge.target}</td>
                          <td className="p-2"><Badge variant="outline">{edge.confidence}</Badge></td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </TabsContent>
            </Tabs>
          </section>
        )}
      </main>
    </div>
  );
}

function sectionName(model: CallflowViewModel, id: string): string {
  return model.sections.find((section) => section.id === id)?.name ?? id;
}
