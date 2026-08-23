use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use compass_agent_graph::{
    AgentGraphError, AgentGraphLimits, ChangeBatch, CompositionProfile, OperationPermission,
    OverlayId, OverlayRevisionId, PrincipalId, ReadRequest, ReadResult, RebaseCommitRequest,
    WriteAuthority,
};
use compass_core::{AgentGraphContext, HistoricalAgentGraphContext};
use compass_model::Graph;
use serde::Serialize;

use super::Outcome;

const MAX_REQUEST_BYTES: u64 = 16 * 1024 * 1024;

enum SelectedContext {
    Current(AgentGraphContext),
    Historical(HistoricalAgentGraphContext),
}

impl SelectedContext {
    fn open(options: &Options) -> Result<Self, AgentGraphError> {
        let Some(realization) = &options.realization else {
            return AgentGraphContext::open_current(
                &options.root,
                &options.graph,
                options.state_root.as_deref(),
            )
            .map(Self::Current);
        };
        if options.graph_explicit || options.state_root.is_some() {
            return Err(AgentGraphError::new(
                compass_agent_graph::AgentGraphErrorCode::InvalidInput,
                "--realization cannot be combined with --graph or --state-root",
            ));
        }
        let repository = compass_history::Repository::discover(&options.root).map_err(|error| {
            AgentGraphError::new(
                compass_agent_graph::AgentGraphErrorCode::UnknownBaseGeneration,
                format!("cannot discover historical repository: {error}"),
            )
        })?;
        let history = compass_history::HistoryStore::open_existing(&repository)
            .map_err(|error| {
                AgentGraphError::new(
                    compass_agent_graph::AgentGraphErrorCode::UnknownBaseGeneration,
                    format!("cannot open history store: {error}"),
                )
            })?
            .ok_or_else(|| {
                AgentGraphError::new(
                    compass_agent_graph::AgentGraphErrorCode::UnknownBaseGeneration,
                    "repository has no initialized Compass history store",
                )
            })?;
        HistoricalAgentGraphContext::open_exact(&options.root, &history, realization, None)
            .map(Self::Historical)
    }

    fn base_generation(&self) -> &compass_agent_graph::BaseGenerationId {
        match self {
            Self::Current(context) => context.base_generation(),
            Self::Historical(context) => context.base_generation(),
        }
    }

    fn repository_id(&self) -> &compass_agent_graph::RepositoryId {
        match self {
            Self::Current(context) => context.repository_id(),
            Self::Historical(context) => context.repository_id(),
        }
    }

    fn active_revision(
        &self,
        overlay: &OverlayId,
    ) -> Result<Option<OverlayRevisionId>, AgentGraphError> {
        match self {
            Self::Current(context) => context.active_revision(overlay),
            Self::Historical(context) => context.active_revision(overlay),
        }
    }

    fn read(&self, request: ReadRequest) -> Result<ReadResult, AgentGraphError> {
        match self {
            Self::Current(context) => context.read(request),
            Self::Historical(context) => context.read(request),
        }
    }

    fn apply(
        &self,
        grant: &compass_agent_graph::WriteGrant,
        batch: ChangeBatch,
    ) -> Result<compass_agent_graph::CommitReceipt, AgentGraphError> {
        match self {
            Self::Current(context) => context.apply(grant, batch),
            Self::Historical(context) => context.apply(grant, batch),
        }
    }

    fn commit_rebase(
        &self,
        grant: &compass_agent_graph::WriteGrant,
        request: RebaseCommitRequest,
    ) -> Result<compass_agent_graph::CommitReceipt, AgentGraphError> {
        match self {
            Self::Current(context) => context.commit_rebase(grant, request),
            Self::Historical(context) => context.commit_rebase(grant, request),
        }
    }
}

pub(super) fn command(args: &[String]) -> Outcome {
    let Some(subcommand) = args.first() else {
        return Outcome::failure_with_code("error: missing agent-graph subcommand".to_owned(), 2);
    };
    if !matches!(
        subcommand.as_str(),
        "status"
            | "apply"
            | "show"
            | "history"
            | "diff"
            | "query"
            | "export"
            | "audit"
            | "rebase-plan"
            | "rebase-commit"
    ) {
        return Outcome::failure_with_code(
            format!("error: unknown agent-graph subcommand {subcommand}"),
            2,
        );
    }
    let json_errors = requested_json(&args[1..]);
    let parsed = match Options::parse(subcommand, &args[1..]) {
        Ok(parsed) => parsed,
        Err(error) => {
            if json_errors {
                let error = AgentGraphError::new(
                    compass_agent_graph::AgentGraphErrorCode::InvalidInput,
                    error,
                );
                let encoded = serde_json::to_string(&error).unwrap_or_else(|_| error.to_string());
                return Outcome::failure_with_code(encoded, 2);
            }
            return Outcome::failure_with_code(format!("error: {error}"), 2);
        }
    };
    let json_errors = matches!(parsed.format, Format::Json);
    match run(subcommand, parsed) {
        Ok(outcome) => outcome,
        Err(error) => agent_error(error, json_errors),
    }
}

fn run(subcommand: &str, options: Options) -> Result<Outcome, AgentGraphError> {
    let context = SelectedContext::open(&options)?;
    match subcommand {
        "status" => {
            let revision = context.active_revision(&options.overlay)?;
            output(
                &AgentGraphStatus {
                    schema: "compass.agent-graph.status/1",
                    overlay: options.overlay,
                    base_generation: context.base_generation().clone(),
                    active_revision: revision,
                    writes_enabled: options.enable_writes,
                },
                options.format,
            )
        }
        "apply" => apply(&context, options),
        "show" => show(&context, options),
        "history" => history(&context, options),
        "diff" => diff(&context, options),
        "query" => query(&context, options),
        "export" => export(&context, options),
        "audit" => audit(&context, options),
        "rebase-plan" => rebase_plan(&context, options),
        "rebase-commit" => rebase_commit(&context, options),
        _ => unreachable!("subcommands are validated before context loading"),
    }
}

fn apply(context: &SelectedContext, options: Options) -> Result<Outcome, AgentGraphError> {
    let request = options.request.as_deref().ok_or_else(|| {
        AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::InvalidInput,
            "apply requires --request FILE",
        )
    })?;
    let bytes = read_request(request)?;
    let batch = serde_json::from_slice::<ChangeBatch>(&bytes).map_err(|error| {
        AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::InvalidInput,
            format!("request is not a strict change batch: {error}"),
        )
    })?;
    if batch.overlay != options.overlay || &batch.base_generation != context.base_generation() {
        return Err(AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::Unauthorized,
            "batch overlay or Base Generation does not match the selected local context",
        ));
    }
    let grant = write_grant(
        context,
        &options,
        batch.overlay.clone(),
        batch.base_generation.clone(),
        batch.expected_revision.clone(),
        false,
    )?;
    let receipt = context.apply(&grant, batch)?;
    output(&receipt, options.format)
}

fn read_request(request: &std::path::Path) -> Result<Vec<u8>, AgentGraphError> {
    let metadata = fs::symlink_metadata(request).map_err(|error| {
        AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::InvalidInput,
            format!("cannot inspect request file: {error}"),
        )
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::InvalidInput,
            "request must be a non-symlink regular file",
        ));
    }
    if metadata.len() > MAX_REQUEST_BYTES {
        return Err(AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::LimitExceeded,
            "request exceeds the 16 MiB hard ceiling",
        ));
    }
    fs::read(request).map_err(|error| {
        AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::InvalidInput,
            format!("cannot read request file: {error}"),
        )
    })
}

fn write_grant(
    context: &SelectedContext,
    options: &Options,
    overlay: OverlayId,
    base_generation: compass_agent_graph::BaseGenerationId,
    expected_revision: Option<OverlayRevisionId>,
    rebase: bool,
) -> Result<compass_agent_graph::WriteGrant, AgentGraphError> {
    let authority = if options.enable_writes {
        WriteAuthority::explicitly_enabled(context.repository_id().clone())
    } else {
        WriteAuthority::disabled(context.repository_id().clone())
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            AgentGraphError::new(
                compass_agent_graph::AgentGraphErrorCode::Unauthorized,
                format!("system clock is invalid: {error}"),
            )
        })?
        .as_secs();
    let mut permissions = BTreeSet::from([
        OperationPermission::PutAssertion,
        OperationPermission::RetractAssertion,
        OperationPermission::PutChallenge,
        OperationPermission::RetractChallenge,
    ]);
    if rebase {
        permissions.insert(OperationPermission::CommitRebase);
    }
    authority.mint_attested(
        options.principal.clone(),
        overlay,
        base_generation,
        expected_revision,
        permissions,
        options.allow_masks,
        now.saturating_add(300),
        AgentGraphLimits::default(),
        compass_agent_graph::WriteAttestation::new("cli")?,
    )
}

fn rebase_plan(context: &SelectedContext, options: Options) -> Result<Outcome, AgentGraphError> {
    let source_revision = options.revision.clone().ok_or_else(|| {
        AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::InvalidInput,
            "rebase-plan requires --revision SOURCE_REVISION",
        )
    })?;
    let result = context.read(ReadRequest::PrepareRebase {
        overlay: options.overlay,
        source_revision,
        target_base_generation: context.base_generation().clone(),
    })?;
    let ReadResult::RebasePlan(plan) = result else {
        return Err(unexpected_result());
    };
    output(&plan, options.format)
}

fn rebase_commit(context: &SelectedContext, options: Options) -> Result<Outcome, AgentGraphError> {
    let request_path = options.request.clone().ok_or_else(|| {
        AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::InvalidInput,
            "rebase-commit requires --request FILE",
        )
    })?;
    let request = serde_json::from_slice::<RebaseCommitRequest>(&read_request(&request_path)?)
        .map_err(|error| {
            AgentGraphError::new(
                compass_agent_graph::AgentGraphErrorCode::InvalidInput,
                format!("request is not a strict rebase commit: {error}"),
            )
        })?;
    if request.plan.overlay != options.overlay
        || &request.plan.target_base_generation != context.base_generation()
    {
        return Err(AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::Unauthorized,
            "rebase plan overlay or target Base Generation does not match the selected context",
        ));
    }
    let grant = write_grant(
        context,
        &options,
        request.plan.overlay.clone(),
        request.plan.target_base_generation.clone(),
        Some(request.plan.source_revision.clone()),
        true,
    )?;
    let receipt = context.commit_rebase(&grant, request)?;
    output(&receipt, options.format)
}

fn show(context: &SelectedContext, options: Options) -> Result<Outcome, AgentGraphError> {
    let assertion = options.positionals.first().ok_or_else(|| {
        AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::InvalidInput,
            "show requires an exact Assertion ID",
        )
    })?;
    let assertion = compass_agent_graph::AssertionId::parse(assertion.clone())?;
    let result = context.read(ReadRequest::Overlay {
        overlay: options.overlay,
        revision: options.revision,
    })?;
    let ReadResult::Overlay { state, .. } = result else {
        return Err(unexpected_result());
    };
    let record = state.assertions.get(&assertion).ok_or_else(|| {
        AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::AssertionNotFound,
            "assertion is not active at the selected revision",
        )
    })?;
    output(record, options.format)
}

fn history(context: &SelectedContext, options: Options) -> Result<Outcome, AgentGraphError> {
    let result = context.read(ReadRequest::History {
        overlay: options.overlay,
        limit: options.limit,
    })?;
    let ReadResult::History(history) = result else {
        return Err(unexpected_result());
    };
    output(&history, options.format)
}

fn audit(context: &SelectedContext, options: Options) -> Result<Outcome, AgentGraphError> {
    let revision = options.revision.ok_or_else(|| {
        AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::InvalidInput,
            "audit requires --revision REVISION",
        )
    })?;
    let ReadResult::Audit(record) = context.read(ReadRequest::Audit { revision })? else {
        return Err(unexpected_result());
    };
    output(&record, options.format)
}

fn diff(context: &SelectedContext, options: Options) -> Result<Outcome, AgentGraphError> {
    if options.positionals.len() != 2 {
        return Err(AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::InvalidInput,
            "diff requires exact OLD and NEW revision digests",
        ));
    }
    let old = parse_revision(&options.positionals[0])?;
    let new = parse_revision(&options.positionals[1])?;
    let result = context.read(ReadRequest::Diff {
        overlay: options.overlay,
        old,
        new,
    })?;
    let ReadResult::Diff(diff) = result else {
        return Err(unexpected_result());
    };
    output(&diff, options.format)
}

fn query(context: &SelectedContext, mut options: Options) -> Result<Outcome, AgentGraphError> {
    let effective = read_effective(context, &options)?;
    let legacy = effective.graph.into_legacy_document().map_err(|error| {
        AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::CorruptOverlay,
            format!("cannot project Effective Graph for query: {error}"),
        )
    })?;
    let graph = Graph::from_document(legacy).map_err(|error| {
        AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::CorruptOverlay,
            format!("cannot hydrate Effective Graph for query: {error}"),
        )
    })?;
    if !options
        .query_args
        .iter()
        .any(|argument| argument == "--cql")
    {
        options.query_args.insert(0, "--cql".to_owned());
    }
    Ok(super::query_commands::command_cql_on_graph(
        &options.query_args,
        &graph,
    ))
}

fn export(context: &SelectedContext, options: Options) -> Result<Outcome, AgentGraphError> {
    let output_path = options.output.clone().ok_or_else(|| {
        AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::InvalidInput,
            "export requires --output PATH",
        )
    })?;
    let output_path = confined_output(&options.root, &output_path)?;
    let effective = read_effective(context, &options)?;
    compass_files::write_json_atomic_new(&output_path, &effective, true).map_err(|error| {
        AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::StorageFailure,
            format!("cannot atomically export Effective Graph: {error}"),
        )
    })?;
    output(
        &serde_json::json!({
            "schema": "compass.agent-graph.export-receipt/1",
            "effectiveIdentity": effective.effective_identity,
            "output": output_path,
        }),
        options.format,
    )
}

fn read_effective(
    context: &SelectedContext,
    options: &Options,
) -> Result<compass_agent_graph::EffectiveGraph, AgentGraphError> {
    let revision = options.revision.clone().ok_or_else(|| {
        AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::InvalidInput,
            "query/export requires --revision DIGEST",
        )
    })?;
    let result = context.read(ReadRequest::EffectiveGraph {
        overlay: options.overlay.clone(),
        revision,
        profile: options.profile,
    })?;
    let ReadResult::EffectiveGraph(effective) = result else {
        return Err(unexpected_result());
    };
    Ok(effective)
}

fn output(value: &impl Serialize, format: Format) -> Result<Outcome, AgentGraphError> {
    let json = serde_json::to_string_pretty(value).map_err(|error| {
        AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::CorruptOverlay,
            format!("cannot encode command result: {error}"),
        )
    })?;
    match format {
        Format::Json => Ok(Outcome::success(json)),
        Format::Text => Ok(Outcome::success(json)),
    }
}

fn agent_error(error: AgentGraphError, json: bool) -> Outcome {
    if json {
        let message = serde_json::to_string(&error).unwrap_or_else(|_| error.to_string());
        Outcome::failure(message)
    } else {
        Outcome::failure(format!("error: {error}"))
    }
}

fn requested_json(args: &[String]) -> bool {
    args.windows(2)
        .any(|pair| pair[0] == "--format" && pair[1] == "json")
}

fn confined_output(
    root: &std::path::Path,
    output: &std::path::Path,
) -> Result<PathBuf, AgentGraphError> {
    let root = root.canonicalize().map_err(|error| {
        AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::InvalidInput,
            format!("cannot canonicalize project root: {error}"),
        )
    })?;
    let absolute = if output.is_absolute() {
        output.to_path_buf()
    } else {
        root.join(output)
    };
    let parent = absolute.parent().ok_or_else(|| {
        AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::InvalidInput,
            "output path has no parent directory",
        )
    })?;
    let parent = parent.canonicalize().map_err(|error| {
        AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::InvalidInput,
            format!("cannot canonicalize output parent: {error}"),
        )
    })?;
    if !parent.starts_with(&root) {
        return Err(AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::Unauthorized,
            "output path must remain beneath the canonical project root",
        ));
    }
    let name = absolute.file_name().ok_or_else(|| {
        AgentGraphError::new(
            compass_agent_graph::AgentGraphErrorCode::InvalidInput,
            "output path must name a file",
        )
    })?;
    Ok(parent.join(name))
}

fn unexpected_result() -> AgentGraphError {
    AgentGraphError::new(
        compass_agent_graph::AgentGraphErrorCode::CorruptOverlay,
        "agent graph repository returned an unexpected result variant",
    )
}

fn parse_revision(value: &str) -> Result<OverlayRevisionId, AgentGraphError> {
    Ok(OverlayRevisionId(compass_agent_graph::Digest::parse(
        value.to_owned(),
    )?))
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Format {
    Text,
    Json,
}

struct Options {
    graph: PathBuf,
    graph_explicit: bool,
    root: PathBuf,
    state_root: Option<PathBuf>,
    realization: Option<compass_history::RealizationId>,
    overlay: OverlayId,
    revision: Option<OverlayRevisionId>,
    profile: CompositionProfile,
    format: Format,
    request: Option<PathBuf>,
    output: Option<PathBuf>,
    principal: PrincipalId,
    enable_writes: bool,
    allow_masks: bool,
    limit: usize,
    positionals: Vec<String>,
    query_args: Vec<String>,
}

impl Options {
    fn parse(subcommand: &str, args: &[String]) -> Result<Self, String> {
        let mut options = Self {
            graph: compass_core::default_graph_path(),
            graph_explicit: false,
            root: std::env::current_dir().map_err(|error| error.to_string())?,
            state_root: None,
            realization: None,
            overlay: OverlayId::parse("overlay:default").map_err(|error| error.to_string())?,
            revision: None,
            profile: CompositionProfile::Augment,
            format: Format::Text,
            request: None,
            output: None,
            principal: PrincipalId::parse("principal:local").map_err(|error| error.to_string())?,
            enable_writes: false,
            allow_masks: false,
            limit: 100,
            positionals: Vec::new(),
            query_args: Vec::new(),
        };
        let mut seen = BTreeSet::new();
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--graph" => {
                    unique(&mut seen, "--graph")?;
                    options.graph = PathBuf::from(required(args, index, "--graph")?);
                    options.graph_explicit = true;
                    index += 2;
                }
                "--root" => {
                    unique(&mut seen, "--root")?;
                    options.root = PathBuf::from(required(args, index, "--root")?);
                    index += 2;
                }
                "--state-root" => {
                    unique(&mut seen, "--state-root")?;
                    options.state_root =
                        Some(PathBuf::from(required(args, index, "--state-root")?));
                    index += 2;
                }
                "--realization" => {
                    unique(&mut seen, "--realization")?;
                    options.realization = Some(
                        required(args, index, "--realization")?
                            .parse::<compass_history::RealizationId>()
                            .map_err(|error| format!("invalid --realization: {error}"))?,
                    );
                    index += 2;
                }
                "--overlay" => {
                    unique(&mut seen, "--overlay")?;
                    options.overlay = OverlayId::parse(required(args, index, "--overlay")?)
                        .map_err(|error| error.to_string())?;
                    index += 2;
                }
                "--revision" => {
                    unique(&mut seen, "--revision")?;
                    options.revision = Some(
                        parse_revision(required(args, index, "--revision")?)
                            .map_err(|error| error.to_string())?,
                    );
                    index += 2;
                }
                "--profile" => {
                    unique(&mut seen, "--profile")?;
                    options.profile = match required(args, index, "--profile")? {
                        "augment" => CompositionProfile::Augment,
                        "curated" => CompositionProfile::Curated,
                        value => return Err(format!("unknown composition profile {value}")),
                    };
                    index += 2;
                }
                "--format" => {
                    unique(&mut seen, "--format")?;
                    options.format = match required(args, index, "--format")? {
                        "text" => Format::Text,
                        "json" => Format::Json,
                        value => return Err(format!("unknown output format {value}")),
                    };
                    index += 2;
                }
                "--request" => {
                    unique(&mut seen, "--request")?;
                    options.request = Some(PathBuf::from(required(args, index, "--request")?));
                    index += 2;
                }
                "--output" => {
                    unique(&mut seen, "--output")?;
                    options.output = Some(PathBuf::from(required(args, index, "--output")?));
                    index += 2;
                }
                "--principal" => {
                    unique(&mut seen, "--principal")?;
                    options.principal = PrincipalId::parse(required(args, index, "--principal")?)
                        .map_err(|error| error.to_string())?;
                    index += 2;
                }
                "--limit" => {
                    unique(&mut seen, "--limit")?;
                    options.limit = required(args, index, "--limit")?
                        .parse::<usize>()
                        .map_err(|_| "--limit must be an integer".to_owned())?;
                    index += 2;
                }
                "--enable-writes" => {
                    unique(&mut seen, "--enable-writes")?;
                    options.enable_writes = true;
                    index += 1;
                }
                "--allow-masks" => {
                    unique(&mut seen, "--allow-masks")?;
                    options.allow_masks = true;
                    index += 1;
                }
                argument if argument.starts_with('-') => {
                    if subcommand == "query" {
                        options.query_args.extend_from_slice(&args[index..]);
                        break;
                    }
                    return Err(format!("unknown option {argument}"));
                }
                positional => {
                    options.positionals.push(positional.to_owned());
                    index += 1;
                }
            }
        }
        if options.allow_masks && !options.enable_writes {
            return Err("--allow-masks requires --enable-writes".to_owned());
        }
        Ok(options)
    }
}

fn unique(seen: &mut BTreeSet<&'static str>, option: &'static str) -> Result<(), String> {
    if !seen.insert(option) {
        return Err(format!("option {option} may only be supplied once"));
    }
    Ok(())
}

fn required<'a>(args: &'a [String], index: usize, option: &str) -> Result<&'a str, String> {
    args.get(index + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("{option} requires a value"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentGraphStatus {
    schema: &'static str,
    overlay: OverlayId,
    base_generation: compass_agent_graph::BaseGenerationId,
    active_revision: Option<OverlayRevisionId>,
    writes_enabled: bool,
}
