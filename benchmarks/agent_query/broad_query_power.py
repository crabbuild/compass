"""Prospective finite-control NI bounds; never consumes product observations."""
import argparse
from datetime import datetime
import hashlib
import json
import math
from pathlib import Path

from benchmarks.agent_query.broad_query_oracle import publish_exclusive
from benchmarks.agent_query.community_tasks import read_bounded
from benchmarks.agent_query.broad_query_review import _binomial_cdf, _binomial_upper

HERE = Path(__file__).resolve().parent


def critical_adverse(total, margin=.02):
    low, high = -1, total
    while high-low > 1:
        middle = (low+high)//2
        if _binomial_upper(middle, total, .025) < margin:
            low = middle
        else:
            high = middle
    return low


def proposal(frozen_at):
    timestamp = datetime.fromisoformat(frozen_at.replace('Z', '+00:00'))
    if timestamp.utcoffset() is None or timestamp.utcoffset().total_seconds() != 0:
        raise ValueError('freeze must be UTC')
    candidates = json.loads(read_bounded(HERE/'broad_query_expansion_sources.json', 4*1024*1024))
    rows = []
    for total in [16,32,106,107,149,183,184,192,200,208,400,512,560,600,640,800]:
        critical = critical_adverse(total)
        rows.append(dict(independentUnits=total, zeroAdverseJointUpper=_binomial_upper(0,total,.025),
            maximumAdverseForStrictTwoPointNI=critical,
            powerIfNoDefiniteWins=[dict(adverseProbability=p,
                probabilityOfPassingFiniteBound=_binomial_cdf(critical,total,p) if critical>=0 else 0.0)
                for p in [0.0,.0025,.005,.01,.015,.02]]))
    source_hashes = {name:hashlib.sha256(read_bounded(HERE/name,4*1024*1024)).hexdigest() for name in
        ['broad_query_power.py','broad_query_expansion_sources.json','broad_query_review.py']}
    return dict(schema='compass.broad-query-power-expansion-proposal/1', frozenAt=frozen_at,
        status='design_only_not_an_executable_task_registration', qualificationEligible=False,
        sourceSupportSha256=source_hashes,
        preservedGates=dict(nonInferiorityMargin=-.02,confidence=.95, firstPageImprovement=.10,
            improvementLowerBoundMustExceed=0,graphAcceptedJudgments=2000,
            graphPrecision=.995,graphWilsonLower=.99),
        finiteBound=dict(method='exact_paired_task_cluster_risk_bonferroni/1',
            adverseUpperFormula='1 - alpha ** (1/n) for zero adverse clusters',
            alphaPerRisk=.025,zeroAdverseStrictNIMinimum=183,
            oneSided95TheoreticalMinimum=149,
            balancedEightRepositoryMinimum=184,
            note='149 uses alpha=.05 and is NOT the implemented joint-bound requirement. 183 is only the optimistic no-loss feasibility floor, not a power guarantee.'),
        scenarioAssumptions=['independent, equally weighted source-task clusters conditional on the fixed repository panel',
            'scenario grid credits no definite wins; partial gains are ignored and any negative within-cluster score counts as an adverse cluster',
            'equal independent-cluster quotas per repository for the implemented finite bound',
            'the hierarchical repository bootstrap may widen intervals beyond this bound',
            'scenario probabilities are assumptions, not inferred from measurements'],
        finiteScenarios=rows,
        initialPanel=dict(tasks=160,broadTasks=80,exactTasks=16,directedTasks=16,
            sourceTaskClusters=106,diagnosticOnlyForTwoPointNI=True,
            note='Exact plus directed labels are at most32 nominal controls; shared identities reduce independent units further. Even107 independent all-tie units would fail the joint two-point bound.'),
        expansion=dict(candidateRepositories=candidates['candidates'], languages=4,repositoriesPerLanguage=4,
            minimumIndependentExactDirectedControlClusters=192,
            desiredQuotaPerRepository=dict(exactControlClusters=9,directedControlClusters=3),
            plannedAdditionalControlLabels=192,
            additionalTaskOraclesFrozen=False, independentReviewCompleted=False,
            equalityAlternative=dict(status='unsupported_not_used_for_power_or_qualification',
                rationale='Unchanged Compass exact/directed functions establish only within-product regression evidence; they do not establish zero loss or discordance against Graphify. No cross-tool p=0 premise is certified.',
                sourceRegressionEvidence='Useful separately for command conformance, never a certificate of equality with the comparator.'),
            developmentPlanningRule=dict(status='pending_development_evidence', targetPower=.9,
                evidence='Only already declared development repositories/tasks may estimate cross-tool discordance and design effect. Unchanged-function source evidence is command conformance only. No held-out output may set assumptions.',
                requiredControls='The planned192 distinct source-task clusters remain exploratory unless prospective development discordance/design effect support the power gate. At adverse probability .005 with no definite wins,192 independent units have about38.2% power, below80% and90% targets.',
                requiredBroad='Plan broad success and broad source-claim precision separately from development paired distributions and the unchanged quality gates;800 is an illustrative scenario only, not a required universal sample size.',
                positiveLift='Positive paired precision lift may establish broad source-claim NI at smaller N; do not impose the all-tie control floor on that metric.'),
            sourceOnlyAdaptationRule='Before any expansion output, inspect candidate sources. If12 independent control responsibility clusters per repository cannot be defended or development planning needs more, expand the untouched candidate pool in a linked pre-output revision; never manufacture independence with paraphrases, overloads, aliases, duplicate bodies or test-name templates.',
            samplingRule='Choose distinct source declarations and receiver/import-bound calls across API/data/errors/lifecycle/ownership boundaries. Exact controls intentionally name identities but require distinct runtime bodies. No body/endpoint is reused across control clusters; collapse shared mechanisms conservatively.',
            allocationRule='Keep equal source-task-cluster counts per repository and language for each metric; preserve initial160tasks and failures, and report exact and directed strata alongside the combined predeclared control metric.',
            stoppingRule='Freeze all expanded questions, source facts, grades, exclusions, clusters, arms, budgets and planning evidence before that panel is captured. Complete the planned panel or report incomplete; no optional stopping after a favorable subset.',
            distinction='Combined exact/directed-control NI and broad source-claim precision NI remain separate. Passing one cannot substitute for the other.'),
        graphEvidenceReuse=dict(allowedOnlyIf='Existing source qualification is bound to the exact current graph relationship-content digest and the same pinned source/profile, source judgments and occurrence/provenance contract.',
            currentNewPanelReusableEvidence='none demonstrated',
            prohibited='A prior tool version, different graph hash, source pin, profile or line-only witness panel is not2000 accepted current graph judgments.'),
        unresolved=['candidate source inventories and actual task/fact oracles not authored or independently reviewed',
            'source-only behavior independence/capacity not established for candidate repositories',
            'the800-unit97.89% scenario is illustrative only; no cross-tool equality alternative is supported;192 independent units do not reach80% power at adverse probability.005 with no definite wins',
            'claim precision is not generally binary; actual score distribution and hierarchical uncertainty can remain inconclusive',
            'no expanded tool outputs, graph judgments, costs or empirical power have been collected'])


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--frozen-at',required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    result=proposal(args.frozen_at)
    data=(json.dumps(result,indent=2)+'\n').encode()
    publish_exclusive(args.output,data)
    print(json.dumps(dict(sha256=hashlib.sha256(data).hexdigest(),status=result['status'])))


if __name__=='__main__':
    main()
