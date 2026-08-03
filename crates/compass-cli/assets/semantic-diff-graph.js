(() => {
  "use strict";

  const NS = "http://www.w3.org/2000/svg";
  const MAX_VISUAL_NODES = 42;
  const MAX_VISUAL_EDGES = 100;
  const MAX_MODEL_NODES = 10000;
  const MAX_MODEL_EDGES = 200000;
  const STATUS_PRIORITY = Object.freeze({
    context: 0,
    added: 1,
    removed: 2,
    changed: 3
  });
  const STATUS_MARK = Object.freeze({
    context: "·",
    added: "+",
    removed: "−",
    changed: "~"
  });

  function element(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  function svgElement(tag, className) {
    const node = document.createElementNS(NS, tag);
    if (className) node.setAttribute("class", className);
    return node;
  }

  function richer(current, candidate) {
    return current || candidate || "";
  }

  function isRecord(value) {
    return typeof value === "object" && value !== null && !Array.isArray(value);
  }

  function validateGraphReport(report) {
    if (!isRecord(report) || report.schema !== "compass.semantic_diff.report/1") {
      throw new Error("Unsupported semantic diff report schema");
    }
    if (!Array.isArray(report.findings) || !Array.isArray(report.source_changes)) {
      throw new Error("Malformed semantic diff evidence");
    }
    const delta = report.graph_delta;
    if (!isRecord(delta)) throw new Error("Missing graph delta");
    const nodeGroups = ["changed_nodes", "removed_nodes", "added_nodes"];
    const edgeGroups = ["changed_edges", "removed_edges", "added_edges"];
    let nodeCount = 0;
    let edgeCount = 0;
    for (const key of nodeGroups) {
      const records = delta[key];
      if (!Array.isArray(records)) throw new Error(`Malformed ${key}`);
      nodeCount += records.length;
      for (const node of records) {
        if (!isRecord(node)
          || typeof node.id !== "string"
          || node.id.length === 0
          || typeof node.label !== "string"
          || typeof node.kind !== "string"
          || typeof node.source_file !== "string"
          || !Array.isArray(node.changed_fields)
          || !node.changed_fields.every((field) => typeof field === "string")) {
          throw new Error(`Malformed ${key} record`);
        }
      }
    }
    for (const key of edgeGroups) {
      const records = delta[key];
      if (!Array.isArray(records)) throw new Error(`Malformed ${key}`);
      edgeCount += records.length;
      for (const edge of records) {
        if (!isRecord(edge)
          || typeof edge.source !== "string"
          || edge.source.length === 0
          || typeof edge.target !== "string"
          || edge.target.length === 0
          || typeof edge.relation !== "string"
          || typeof edge.key !== "string"
          || typeof edge.source_file !== "string"
          || !Array.isArray(edge.changed_fields)
          || !edge.changed_fields.every((field) => typeof field === "string")) {
          throw new Error(`Malformed ${key} record`);
        }
      }
    }
    if (nodeCount > MAX_MODEL_NODES || edgeCount > MAX_MODEL_EDGES) {
      throw new Error("Graph delta exceeds the interactive safety limit");
    }
    return delta;
  }

  function rememberNode(nodes, raw, status) {
    const source = typeof raw === "string" ? { id: raw } : raw || {};
    const id = source.id;
    if (!id) return;
    const existing = nodes.get(id);
    if (!existing) {
      nodes.set(id, {
        id,
        label: source.label || id,
        kind: source.kind || "",
        sourceFile: source.source_file || "",
        changedFields: [...(source.changed_fields || [])],
        status,
        degree: 0
      });
      return;
    }
    existing.label = richer(
      existing.label === existing.id ? "" : existing.label,
      source.label
    ) || id;
    existing.kind = richer(existing.kind, source.kind);
    existing.sourceFile = richer(existing.sourceFile, source.source_file);
    if (source.changed_fields?.length) {
      existing.changedFields = [...new Set([
        ...existing.changedFields,
        ...source.changed_fields
      ])].sort();
    }
    if (STATUS_PRIORITY[status] > STATUS_PRIORITY[existing.status]) {
      existing.status = status;
    }
  }

  function buildGraphModel(report) {
    const delta = validateGraphReport(report);
    const nodes = new Map();
    for (const node of delta.changed_nodes || []) rememberNode(nodes, node, "changed");
    for (const node of delta.removed_nodes || []) rememberNode(nodes, node, "removed");
    for (const node of delta.added_nodes || []) rememberNode(nodes, node, "added");
    const edges = [
      ...(delta.changed_edges || []).map((edge) => ({ ...edge, status: "changed" })),
      ...(delta.removed_edges || []).map((edge) => ({ ...edge, status: "removed" })),
      ...(delta.added_edges || []).map((edge) => ({ ...edge, status: "added" }))
    ].filter((edge) => edge.source && edge.target);
    for (const edge of edges) {
      rememberNode(nodes, edge.source, "context");
      rememberNode(nodes, edge.target, "context");
      if (nodes.size > MAX_MODEL_NODES) {
        throw new Error("Graph delta exceeds the interactive node safety limit");
      }
      nodes.get(edge.source).degree += 1;
      nodes.get(edge.target).degree += 1;
    }
    return { nodes, edges };
  }

  function rankVisualNodes(model) {
    const rankWithinStatus = (left, right) => {
      const degreeDifference = right.degree - left.degree;
      return degreeDifference || left.id.localeCompare(right.id);
    };
    const nodes = [...model.nodes.values()];
    const buckets = ["changed", "added", "removed"].map((status) =>
      nodes.filter((node) => node.status === status).sort(rankWithinStatus)
    );
    const ranked = [];
    for (let index = 0; buckets.some((bucket) => index < bucket.length); index += 1) {
      for (const bucket of buckets) {
        if (index < bucket.length) ranked.push(bucket[index]);
      }
    }
    ranked.push(
      ...nodes.filter((node) => node.status === "context").sort(rankWithinStatus)
    );
    return ranked;
  }

  function displayLabel(node) {
    return node.label || node.id;
  }

  function truncate(value, max) {
    return value.length > max ? `${value.slice(0, max - 1)}…` : value;
  }

  function basename(value) {
    const normalized = value.replaceAll("\\", "/");
    return normalized.split("/").filter(Boolean).at(-1) || normalized;
  }

  function capsuleWidth(node) {
    const visible = truncate(displayLabel(node), 21);
    return Math.max(118, Math.min(172, 46 + visible.length * 6.25));
  }

  function layoutVisualNodes(nodes, width, height) {
    const columns = Math.max(1, Math.min(5, nodes.length));
    const rows = Math.max(1, Math.ceil(nodes.length / columns));
    const xSpacing = width / columns;
    const ySpacing = height / rows;
    return nodes.map((node, index) => {
      const row = Math.floor(index / columns);
      const offset = index % columns;
      const column = row % 2 === 0 ? offset : columns - offset - 1;
      return {
        ...node,
        width: capsuleWidth(node),
        height: 44,
        x: xSpacing * (column + .5),
        y: ySpacing * (row + .5)
      };
    });
  }

  function edgeEndpoints(source, target) {
    const dx = target.x - source.x;
    const dy = target.y - source.y;
    const distance = Math.max(Math.hypot(dx, dy), 1);
    const ux = dx / distance;
    const uy = dy / distance;
    const sourceInset = Math.min(source.width / 2, 24 / Math.max(Math.abs(uy), .2));
    const targetInset = Math.min(target.width / 2, 24 / Math.max(Math.abs(uy), .2));
    return {
      x1: source.x + ux * sourceInset,
      y1: source.y + uy * sourceInset,
      x2: target.x - ux * (targetInset + 7),
      y2: target.y - uy * (targetInset + 7)
    };
  }

  function markerDefinitions() {
    const definitions = svgElement("defs");
    for (const status of ["added", "removed", "changed", "context"]) {
      const marker = svgElement("marker");
      marker.id = `arrow-${status}`;
      marker.setAttribute("viewBox", "0 0 10 10");
      marker.setAttribute("refX", "8");
      marker.setAttribute("refY", "5");
      marker.setAttribute("markerWidth", "5");
      marker.setAttribute("markerHeight", "5");
      marker.setAttribute("orient", "auto-start-reverse");
      const path = svgElement("path", `graph-arrow ${status}`);
      path.setAttribute("d", "M 0 0 L 10 5 L 0 10 z");
      marker.append(path);
      definitions.append(marker);
    }
    return definitions;
  }

  function nodeAccessibleName(node) {
    const kind = node.kind ? ` ${node.kind}` : "";
    return `${node.status} ${kind} ${displayLabel(node)}`.trim();
  }

  function renderSvg(model, visualNodes, visualEdges, host) {
    const width = 980;
    const height = 560;
    const positioned = layoutVisualNodes(visualNodes, width, height);
    const byId = new Map(positioned.map((node) => [node.id, node]));
    const svg = svgElement("svg");
    svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
    svg.setAttribute("aria-label", "Changed code graph");
    svg.append(markerDefinitions());

    const edgeElements = [];
    for (const edge of visualEdges) {
      const source = byId.get(edge.source);
      const target = byId.get(edge.target);
      if (!source || !target) continue;
      const points = edgeEndpoints(source, target);
      const line = svgElement("line", `graph-edge ${edge.status}`);
      for (const [name, value] of Object.entries(points)) {
        line.setAttribute(name, String(value));
      }
      line.dataset.source = edge.source;
      line.dataset.target = edge.target;
      line.dataset.relation = edge.relation || "";
      line.setAttribute("marker-end", `url(#arrow-${edge.status})`);
      const title = svgElement("title");
      title.textContent =
        `${edge.source} —${edge.relation || "related"}→ ${edge.target}`;
      line.append(title);
      svg.append(line);
      edgeElements.push(line);
    }

    const nodeElements = new Map();
    for (const node of positioned) {
      const group = svgElement("g", `graph-node ${node.status}`);
      group.dataset.nodeId = node.id;
      group.setAttribute("transform", `translate(${node.x} ${node.y})`);
      group.setAttribute("role", "button");
      group.setAttribute("tabindex", "0");
      group.setAttribute("aria-pressed", "false");
      group.setAttribute("aria-label", nodeAccessibleName(node));
      const title = svgElement("title");
      title.textContent = nodeAccessibleName(node);
      const surface = svgElement("rect", "graph-node-surface");
      surface.setAttribute("x", String(-node.width / 2));
      surface.setAttribute("y", "-22");
      surface.setAttribute("width", String(node.width));
      surface.setAttribute("height", "44");
      surface.setAttribute("rx", "8");
      surface.setAttribute("ry", "8");
      const mark = svgElement("text", "graph-node-mark");
      mark.setAttribute("x", String(-node.width / 2 + 13));
      mark.setAttribute("y", "4");
      mark.textContent = STATUS_MARK[node.status];
      const label = svgElement("text", "graph-node-label");
      label.setAttribute("x", String(-node.width / 2 + 31));
      label.setAttribute("y", "-4");
      label.textContent = truncate(displayLabel(node), 21);
      const meta = svgElement("text", "graph-node-meta");
      meta.setAttribute("x", String(-node.width / 2 + 31));
      meta.setAttribute("y", "12");
      const metadata = [node.kind, node.sourceFile && basename(node.sourceFile)]
        .filter(Boolean)
        .join(" · ");
      meta.textContent = truncate(metadata || node.id, 30);
      group.append(title, surface, mark, label, meta);
      svg.append(group);
      nodeElements.set(node.id, group);
    }
    host.replaceChildren(svg);
    return { svg, nodeElements, edgeElements, visualNodeIds: new Set(byId.keys()) };
  }

  function section(title, emptyText) {
    const wrapper = element("section", "graph-inspector-section");
    wrapper.append(element("h4", "", title));
    if (emptyText) wrapper.append(element("p", "", emptyText));
    return wrapper;
  }

  function fact(term, value) {
    const wrapper = element("div");
    wrapper.append(element("dt", "", term), element("dd", "", value));
    return wrapper;
  }

  function relationshipsFor(model, nodeId) {
    return {
      incoming: model.edges.filter((edge) => edge.target === nodeId),
      outgoing: model.edges.filter((edge) => edge.source === nodeId)
    };
  }

  function findingsFor(report, nodeId) {
    return (report.findings || []).filter((finding) =>
      finding.subject === nodeId ||
      (finding.evidence || []).some((evidence) => evidence.record_key === nodeId)
    );
  }

  function normalizePath(value) {
    return (value || "").replaceAll("\\", "/").replace(/^\.\/+/, "");
  }

  function sourceIndexFor(report, sourceFile) {
    const wanted = normalizePath(sourceFile);
    if (!wanted) return -1;
    return (report.source_changes || []).findIndex((change) =>
      normalizePath(change.old_path) === wanted ||
      normalizePath(change.new_path) === wanted
    );
  }

  function relationButton(edge, direction, model) {
    const neighborId = direction === "incoming" ? edge.source : edge.target;
    const neighbor = model.nodes.get(neighborId);
    const button = element("button", "graph-relation");
    button.type = "button";
    button.dataset.neighborId = neighborId;
    button.append(
      element("strong", "", neighbor ? displayLabel(neighbor) : neighborId),
      element(
        "span",
        "",
        `${direction === "incoming" ? "←" : "→"} ${edge.relation || "related"} · ${edge.status}`
      )
    );
    return button;
  }

  function emptyInspector(inspector) {
    const heading = element("h3", "sr-only", "Node inspector");
    heading.id = "graph-inspector-heading";
    inspector.replaceChildren(
      heading,
      element("p", "graph-inspector-empty", "Select a node to inspect its change.")
    );
  }

  function renderInspector(report, model, node, inspector) {
    const header = element("header", "graph-inspector-header");
    const status = element("span", `graph-status ${node.status}`, node.status);
    const heading = element("h3", "", displayLabel(node));
    heading.id = "graph-inspector-heading";
    header.append(status, heading, element("code", "", node.id));

    const facts = element("dl", "graph-inspector-facts");
    if (node.kind) facts.append(fact("Kind", node.kind));
    if (node.sourceFile) facts.append(fact("Source", node.sourceFile));
    if (node.changedFields.length) {
      facts.append(fact("Changed fields", node.changedFields.join(", ")));
    }

    const relationships = relationshipsFor(model, node.id);
    const incoming = section(
      "Incoming relationships",
      relationships.incoming.length ? "" : "No changed incoming relationships."
    );
    for (const edge of relationships.incoming) {
      incoming.append(relationButton(edge, "incoming", model));
    }
    const outgoing = section(
      "Outgoing relationships",
      relationships.outgoing.length ? "" : "No changed outgoing relationships."
    );
    for (const edge of relationships.outgoing) {
      outgoing.append(relationButton(edge, "outgoing", model));
    }

    const related = findingsFor(report, node.id);
    const findings = section(
      "Related findings",
      related.length ? "" : "No related semantic findings."
    );
    for (const finding of related) {
      if (!document.getElementById(finding.id)) continue;
      const link = element("a", "graph-inspector-link", finding.headline || finding.id);
      link.href = `#${finding.id}`;
      findings.append(link);
    }

    const navigation = section("Navigate");
    const sourceIndex = sourceIndexFor(report, node.sourceFile);
    if (sourceIndex >= 0 && document.getElementById(`source-change-${sourceIndex}`)) {
      const sourceLink = element("a", "graph-inspector-link", "View source patch");
      sourceLink.href = `#source-change-${sourceIndex}`;
      navigation.append(sourceLink);
    }
    const listRow = [...document.querySelectorAll("[data-graph-node-id]")]
      .find((row) => row.dataset.graphNodeId === node.id);
    if (listRow) {
      const listButton = element("button", "graph-list-link", "Show in exhaustive list");
      listButton.type = "button";
      listButton.dataset.listNodeId = node.id;
      navigation.append(listButton);
    }
    if (!navigation.querySelector("a,button")) {
      navigation.append(element("p", "", "No exact navigation target is available."));
    }
    inspector.replaceChildren(header, facts, incoming, outgoing, findings, navigation);
  }

  function mount({ report, host, inspector, liveRegion, note }) {
    if (!report || !host || !inspector || !liveRegion || !note) {
      console.warn("Compass could not mount the changed graph");
      return Object.freeze({ clear() {}, select() {}, destroy() {} });
    }
    const listeners = [];
    try {
      const model = buildGraphModel(report);
      const ranked = rankVisualNodes(model);
      const visualNodes = ranked.slice(0, MAX_VISUAL_NODES);
      const selectedIds = new Set(visualNodes.map((node) => node.id));
      const visualEdges = model.edges
        .filter((edge) => selectedIds.has(edge.source) && selectedIds.has(edge.target))
        .sort((left, right) =>
          `${left.source}\0${left.relation || ""}\0${left.target}\0${left.key || ""}`
            .localeCompare(
              `${right.source}\0${right.relation || ""}\0${right.target}\0${right.key || ""}`
            )
        )
        .slice(0, MAX_VISUAL_EDGES);
      const rendered = renderSvg(model, visualNodes, visualEdges, host);
      const defaultNote = note.textContent;
      const truncated =
        ranked.length > visualNodes.length || model.edges.length > visualEdges.length;
      const sampleNote =
        `Visual sample: ${visualNodes.length} of ${model.nodes.size} involved nodes ` +
        `and ${visualEdges.length} of ${model.edges.length} changed edges. ` +
        "The lists below and embedded JSON remain exhaustive.";
      if (truncated) {
        note.textContent = sampleNote;
      }
      emptyInspector(inspector);

      function clear() {
        for (const group of rendered.nodeElements.values()) {
          group.classList.remove("is-selected", "is-neighbor", "is-dimmed");
          group.setAttribute("aria-pressed", "false");
        }
        for (const edge of rendered.edgeElements) {
          edge.classList.remove("is-related", "is-dimmed");
        }
        emptyInspector(inspector);
        note.textContent = truncated ? sampleNote : defaultNote;
        liveRegion.textContent = "Graph selection cleared";
      }

      function select(nodeId) {
        const node = model.nodes.get(nodeId);
        if (!node) return;
        const relationships = relationshipsFor(model, nodeId);
        const neighbors = new Set([
          ...relationships.incoming.map((edge) => edge.source),
          ...relationships.outgoing.map((edge) => edge.target)
        ]);
        for (const [id, group] of rendered.nodeElements) {
          const selected = id === nodeId;
          const neighbor = neighbors.has(id);
          group.classList.toggle("is-selected", selected);
          group.classList.toggle("is-neighbor", neighbor);
          group.classList.toggle("is-dimmed", !selected && !neighbor);
          group.setAttribute("aria-pressed", String(selected));
        }
        for (const edge of rendered.edgeElements) {
          const related =
            edge.dataset.source === nodeId || edge.dataset.target === nodeId;
          edge.classList.toggle("is-related", related);
          edge.classList.toggle("is-dimmed", !related);
        }
        renderInspector(report, model, node, inspector);
        liveRegion.textContent = `Inspecting ${displayLabel(node)}`;
        if (!rendered.visualNodeIds.has(nodeId)) {
          note.textContent =
            "Inspecting a node outside the bounded visual sample. Its changed " +
            "relationships remain available here and in the exhaustive lists.";
        } else if (truncated) {
          note.textContent = sampleNote;
        } else {
          note.textContent = defaultNote;
        }
      }

      const onHostClick = (event) => {
        const node = event.target.closest?.("[data-node-id]");
        if (node && host.contains(node)) {
          select(node.dataset.nodeId);
          return;
        }
        if (event.target === rendered.svg || event.target === host) clear();
      };
      const onHostKeydown = (event) => {
        const node = event.target.closest?.("[data-node-id]");
        if (node && (event.key === "Enter" || event.key === " ")) {
          event.preventDefault();
          select(node.dataset.nodeId);
        }
        if (event.key === "Escape") {
          event.preventDefault();
          clear();
        }
      };
      const onInspectorClick = (event) => {
        const neighbor = event.target.closest?.("[data-neighbor-id]");
        if (neighbor && inspector.contains(neighbor)) {
          select(neighbor.dataset.neighborId);
          return;
        }
        const list = event.target.closest?.("[data-list-node-id]");
        if (!list || !inspector.contains(list)) return;
        const row = [...document.querySelectorAll("[data-graph-node-id]")]
          .find((candidate) => candidate.dataset.graphNodeId === list.dataset.listNodeId);
        if (!row) return;
        row.tabIndex = -1;
        row.classList.add("graph-list-focus");
        row.scrollIntoView({ block: "center" });
        row.focus({ preventScroll: true });
        globalThis.setTimeout(() => row.classList.remove("graph-list-focus"), 1600);
      };
      host.addEventListener("click", onHostClick);
      host.addEventListener("keydown", onHostKeydown);
      inspector.addEventListener("click", onInspectorClick);
      listeners.push(
        () => host.removeEventListener("click", onHostClick),
        () => host.removeEventListener("keydown", onHostKeydown),
        () => inspector.removeEventListener("click", onInspectorClick)
      );
      return Object.freeze({
        clear,
        select,
        destroy() {
          for (const remove of listeners) remove();
          host.replaceChildren();
          inspector.replaceChildren();
        }
      });
    } catch (error) {
      host.replaceChildren(
        element(
          "p",
          "graph-render-fallback",
          "Interactive graph unavailable. Use the exhaustive node and edge lists below."
        )
      );
      emptyInspector(inspector);
      note.textContent =
        "The embedded report data and exhaustive graph-change lists remain available.";
      console.warn("Compass could not render the changed graph", error);
      return Object.freeze({ clear() {}, select() {}, destroy() {} });
    }
  }

  globalThis.CompassSemanticDiffGraph = Object.freeze({ mount });
})();
