"""Bounded initial protocol2 suggestions from original printed bytes only.

No oracle/source registration facts enter this parser. Owner ambiguity, prose,
assertion equivalence and source meaning remain independent review obligations.
All initial suggestions have incomplete semantic coverage, including empty and
native no-match output. Native outcomes/costs belong to the collector separately.
"""
import json
from itertools import chain

from benchmarks.agent_query import broad_query_capture as old
from benchmarks.agent_query.broad_query_coverage import NORMALIZED_SCHEMA
from benchmarks.agent_query.broad_query_protocol import (
    MAX_RAW_BYTES, PrintedSpanBudget, ProtocolLimit, SHA, _claim_id, pack_claim_batches,
)


def _bindings(task, tool, arm, registration_sha, actors):
    _claim_id(dict(id=task))
    if tool not in old.TOOLS or arm not in old.ARMS:
        raise ValueError("unknown native tool or cost arm")
    if type(registration_sha) is not str or not SHA.fullmatch(registration_sha):
        raise ValueError("invalid frozen source registration digest")
    if type(actors) is not dict or not 1 <= len(actors) <= 16:
        raise ValueError("invalid bounded frozen actor tuple")
    for name, digest in actors.items():
        if (type(name) is not str or not old.SAFE_KEY.fullmatch(name)
                or type(digest) is not str or not SHA.fullmatch(digest)):
            raise ValueError("invalid frozen actor name or digest")
    # Full collection/assessment must compare this entire tuple to its frozen
    # registered module set and verify every original/retained module's bytes.
    return dict(schema=NORMALIZED_SCHEMA, task=task, tool=tool, arm=arm,
        sourceRegistrationSha256=registration_sha, actorSha256=dict(sorted(actors.items())))


def _json_size(document):
    """Bound encoded bytes without constructing an oversized JSON document."""
    count = 0
    # Match the collector's actual indent2/sorted/ASCII/newline encoding.
    encoder = json.JSONEncoder(indent=2, ensure_ascii=True, sort_keys=True, allow_nan=False)
    for part in chain(encoder.iterencode(document), ("\n",)):
        requested = count + len(part.encode("utf-8"))
        if requested > old.MAX_JSON_BYTES:
            error = ProtocolLimit("JSON_BYTE_LIMIT", "encoded_json_bytes", old.MAX_JSON_BYTES,
                                  requested, requested)
            error.resource["exactCountKnown"] = False
            raise error
        count = requested


def _endpoint_summaries(labels):
    """One conservative summary per label; total work is linear in records."""
    endpoints = {}
    for label, items in labels.items():
        first = items[0].get("resolvedIdentity")
        endpoints[label] = first if first is not None and all(
            item.get("resolvedIdentity") == first for item in items) else None
    return endpoints


def _parse(output, tool, graph, binding, bundle, failed, limited, no_match):
    budget = PrintedSpanBudget(output, bundle)
    binding = dict(binding, rawStdoutSha256=budget.raw_sha256)
    text_lines = budget._text.split("\n")
    ledger = budget.ledger()
    parsed_lines = []
    for entry in ledger:
        number = entry["physicalLine"]
        line = text_lines[number - 1]
        if number < len(text_lines) and line.endswith("\r"):
            line = line[:-1]
        parsed_lines.append((line, entry["span"]))
    ranked, claims, queue, labels, seen = [], [], [], {}, set()
    declaration_claims, relation_claims = {}, {}
    occurrence_count = 0
    printed_ranks, resolved_ranks = [], []
    line_claims = [[] for _ in ledger]

    def occurrence(claim, span, printed_site=None):
        nonlocal occurrence_count
        if occurrence_count + 1 > 8192:
            raise ProtocolLimit("OCCURRENCE_LIMIT", "registered_occurrence_records",8192,
                                occurrence_count,occurrence_count+1)
        occurrence_count += 1
        item = dict(id=f"occurrence-{occurrence_count:05}",span=span)
        if printed_site is not None:
            item["printedSite"] = printed_site
        for index in budget.join("initial",claim["id"],item["id"],span):
            if claim["id"] not in line_claims[index]:
                line_claims[index].append(claim["id"])
        claim["printedOccurrences"].append(item)

    def new_claim(**fields):
        if len(claims) + 1 > 2048:
            raise ProtocolLimit("CLAIM_LIMIT","registered_claim_records",2048,len(claims),len(claims)+1)
        claim = dict(id=f"claim-{len(claims)+1:04}",printedOccurrences=[],**fields)
        claims.append(claim)
        return claim

    for line, span in parsed_lines:
        match = (old.GRAPHIFY_NODE.match(line) if tool == "graphify" else
            old.COMPASS_EVIDENCE_NODE.match(line) or old.COMPASS_NODE.match(line)
            or old.COMPASS_SEED.match(line) or old.RANKED_NODE.match(line) or old.AGENT_NODE.match(line))
        if not match:
            continue
        rank = match.groupdict().get("rank")
        if rank is not None:
            printed_ranks.append(int(rank))
        try:
            file = old._relative(match["file"])
            lineno, label = int(match["line"]), match["label"]
            identity = old._printed_identity(label,file,lineno)
        except ValueError:
            queue.append(dict(kind="invalid_printed_anchor",span=span))
            continue
        terminal = old._terminal_symbol(label)
        matches = graph.anchors.get((file,lineno,terminal),[]) if terminal else []
        printed_id = match.groupdict().get("id")
        resolved = identity is not None and len(matches) == 1 and (printed_id is None or matches[0] == printed_id)
        item = dict(kind="printed_declaration",printedLabel=label,file=file,startLine=lineno,
                    graphMatchCount=len(matches),span=span,ownerPrinted=identity is not None)
        queue.append(item)
        labels.setdefault(label,[]).append(item)
        if not resolved:
            continue
        item["resolvedIdentity"] = identity.record()
        claim = declaration_claims.get(identity)
        if claim is None:
            claim = new_claim(identity=identity.record(),span=span,assertionKind="declaration_identity_only")
            declaration_claims[identity] = claim
        occurrence(claim,span)
        if identity in seen:
            continue
        if len(ranked) >= 512:
            raise ValueError("printed candidates exceed bounded global ranking")
        seen.add(identity)
        ranked.append(dict(identity=identity.record(),nodeId=matches[0],firstPage=True,span=span))
        if rank is not None:
            resolved_ranks.append(int(rank))

    # Resolve each complete declaration group once. An unresolved/conflicting
    # occurrence poisons that label conservatively, regardless of repetition.
    # Edge processing must not rescan a potentially large group per edge.
    endpoints = _endpoint_summaries(labels)

    for line, span in parsed_lines:
        edge_line, printed_site, malformed = old._printed_edge_site(line)
        edge = (old.GRAPHIFY_EDGE if tool == "graphify" else old.COMPASS_EDGE).match(edge_line)
        if not edge:
            continue
        valid = [endpoints.get(edge[end]) for end in ("source","target")]
        queue.append(dict(kind="printed_relation",sourceLabel=edge["source"],targetLabel=edge["target"],
            relation=edge["kind"],span=span,printedSite=printed_site,malformedPrintedSite=malformed))
        if not all(valid) or malformed is not None:
            continue
        # Different printed occurrence citations stay separate. A source-aware
        # reviewer may later correct this initial partition without rewriting it.
        site_key = tuple(sorted(printed_site.items())) if printed_site is not None else None
        key = old.Identity.parse(valid[0]),old.Identity.parse(valid[1]),edge["kind"],site_key
        claim = relation_claims.get(key)
        if claim is None:
            claim = new_claim(identity=valid[0],span=span,relation=dict(source=valid[0],target=valid[1],
                kind=edge["kind"],direction="source_to_target",occurrences=[]))
            relation_claims[key] = claim
        occurrence(claim,span,printed_site)

    # Queue every remaining line without a quadratic scan over parsed records.
    recognized = {(item["span"]["startByte"],item["span"]["endByte"]) for item in queue}
    for line, span in parsed_lines:
        if (span["startByte"],span["endByte"]) not in recognized:
            queue.append(dict(kind="unparsed_printed_text",span=span))
    claims.sort(key=lambda c:(c["span"]["startByte"],c["span"]["endByte"],c["id"]))
    order = {claim["id"]:index for index,claim in enumerate(claims)}
    for entry, links in zip(ledger,line_claims):
        entry["claimIds"] = sorted(links,key=order.__getitem__)
    queue.sort(key=lambda item:(item["span"]["startByte"],item["span"]["endByte"],item["kind"]))
    declarations = any(item["kind"] == "printed_declaration" for item in queue)
    native_empty = no_match or any(signal in budget._text for signal in (
        "No matching nodes found","No node matching","No supporting indexed evidence found.",
        "No matching indexed","no_match"))
    disposition = "limit" if limited else "error" if failed else "answered" if declarations else "abstain" if native_empty else "error"
    exact_ranking = bool(printed_ranks) and printed_ranks == list(range(1,len(printed_ranks)+1)) and resolved_ranks == printed_ranks and len(ranked) == len(printed_ranks)
    document = dict(binding,normalizationStatus="pending_review",normalizationComplete=False,
        semanticCoverageComplete=False,disposition=disposition,rankingKind="retrieval" if exact_ranking else "display",
        admission=None,ranked=ranked,lineLedger=ledger,reviewQueue=queue,**pack_claim_batches(claims))
    _json_size(document)
    return document


def normalize_v2(output,tool,graph,*,task,arm,source_registration_sha256,actors,
                 bundle=None,failed=False,limited=False,no_match_outcome=False):
    """Return complete initial suggestions or a bounded typed row rejection.

    Invalid frozen bindings are bundle integrity errors and raise. Parser limits
    never become native no-match, semantic empty or a successful claim prefix.
    The collector retains native output/outcome/costs and the entire matrix.
    """
    binding = _bindings(task,tool,arm,source_registration_sha256,actors)
    if type(output) is not bytes or len(output) > MAX_RAW_BYTES:
        raise ValueError("collector must supply bounded immutable native stdout")
    if any(type(flag) is not bool for flag in (failed,limited,no_match_outcome)):
        raise ValueError("native outcome flags must be exact booleans")
    try:
        return _parse(output,tool,graph,binding,bundle,failed,limited,no_match_outcome)
    except (ValueError,UnicodeError) as error:
        code = error.code if isinstance(error,ProtocolLimit) else "INVALID_UTF8" if isinstance(error,UnicodeError) else "PARSER_REJECTION"
        failure = dict(code=code,resource=error.resource if isinstance(error,ProtocolLimit) else None,
                       message="Initial normalization rejected; original native bytes and costs must be retained.")
        return dict(binding,rawStdoutSha256=old.sha(output),normalizationStatus="rejected",
            normalizationComplete=False,semanticCoverageComplete=False,claimCount=0,claimBatches=[],
            printedAssertionCount=None,fullMeasuredPrecision=None,falseEmpty=None,success=False,
            normalizationFailure=failure,disposition="limit" if limited else "error",
            rankingKind="unavailable",admission=None,ranked=[])
