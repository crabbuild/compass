use compass_agent_graph::AgentGraphPaths;

#[test]
fn explicit_state_root_is_absolute_owner_storage() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let state = directory.path().join("agent-state");
    let paths = AgentGraphPaths::for_explicit_state_root(&state)?;
    assert_eq!(paths.root(), state.canonicalize()?);
    assert_eq!(paths.database(), paths.root().join("agent-graph.sqlite3"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlinked_storage_root_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir()?;
    let target = directory.path().join("target");
    std::fs::create_dir(&target)?;
    let alias = directory.path().join("alias");
    symlink(target, &alias)?;
    assert!(AgentGraphPaths::for_explicit_state_root(&alias).is_err());
    Ok(())
}
