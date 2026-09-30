use std::error::Error;
use std::fs::{self, File, FileTimes};
use std::path::Path;

use compass_model::{GraphDocument, GraphError};

type Loader = fn(&Path) -> Result<GraphDocument, GraphError>;

fn preserved_metadata_replacement(loader: Loader) -> Result<(), Box<dyn Error>> {
    for atomic in [false, true] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("graph.json");
        fs::create_dir(directory.path().join("cache"))?;
        let before = r#"{"directed":true,"nodes":[{"id":"a","label":"Alpha"},{"id":"b","label":"Beta"}],"links":[{"source":"a","target":"b","relation":"calls"}]}"#;
        let after = before.replace("Alpha", "Omega").replace("Beta", "Zeta");
        assert_eq!(before.len(), after.len());
        fs::write(&path, before)?;
        let modified = path.metadata()?.modified()?;
        assert_eq!(loader(&path)?.nodes[0].label(), "Alpha");
        assert!(
            fs::read_dir(directory.path().join("cache"))?
                .next()
                .is_some()
        );
        let destination = if atomic {
            directory.path().join("next.json")
        } else {
            path.clone()
        };
        fs::write(&destination, after)?;
        File::options()
            .write(true)
            .open(&destination)?
            .set_times(FileTimes::new().set_modified(modified))?;
        if atomic {
            // NamedTempFile::persist uses the platform's atomic replacement primitive.
            let temporary = tempfile::NamedTempFile::new_in(directory.path())?;
            fs::copy(&destination, temporary.path())?;
            temporary
                .as_file()
                .set_times(FileTimes::new().set_modified(modified))?;
            temporary.persist(&path)?;
        }
        assert_eq!(path.metadata()?.modified()?, modified);
        for _ in 0..2 {
            let document = loader(&path)?;
            assert_eq!(document.nodes[0].label(), "Omega", "atomic={atomic}");
            assert_eq!(document.nodes[1].label(), "Zeta");
            assert_eq!(document.links.len(), 1);
        }
        // A same-length corrupt replacement must not become a successful cache hit.
        fs::write(&path, vec![b'!'; before.len()])?;
        File::options()
            .write(true)
            .open(&path)?
            .set_times(FileTimes::new().set_modified(modified))?;
        assert!(loader(&path).is_err());
        fs::remove_file(&path)?;
        assert!(loader(&path).is_err());
    }
    Ok(())
}

#[test]
fn query_cache_reads_changed_content_with_preserved_metadata() -> Result<(), Box<dyn Error>> {
    preserved_metadata_replacement(GraphDocument::load)
}

#[test]
fn traversal_cache_reads_changed_content_with_preserved_metadata() -> Result<(), Box<dyn Error>> {
    preserved_metadata_replacement(GraphDocument::load_for_traversal)
}

#[test]
fn affected_cache_reads_changed_content_with_preserved_metadata() -> Result<(), Box<dyn Error>> {
    preserved_metadata_replacement(GraphDocument::load_for_affected)
}
