#[test]
fn php_case_folded_calls_traits_inheritance_imports_and_construction_resolve_stably()
-> Result<(), Box<dyn std::error::Error>> {
    let helpers_source = br#"<?php
namespace Helpers;
function Assist(): void {}
function GlobalOnly(): void {}
"#;
    let global_source = br#"<?php
function GlobalHelp(): void {}
"#;
    let worker_source = br#"<?php
namespace App;

use function Helpers\Assist as HELP;

trait Logs { public function Write(): void {} }
class BaseWorker { public function ParentWork(): void {} }
class Worker extends BaseWorker {
    use Logs;
    public function LocalWork(): void {}
    public function Execute(): void {
        $this->wRiTe();
        $this->PARENTWORK();
        self::localwork();
        help();
        GLOBALHELP();
    }
}

function build(): Worker { return new WORKER(); }
"#;
    let helpers = extract("src/helpers.php", helpers_source);
    let global = extract("src/global.php", global_source);
    let worker = extract("src/Worker.php", worker_source);
    let sources = HashMap::from([
        (
            "src/helpers.php".to_owned(),
            String::from_utf8_lossy(helpers_source).into_owned(),
        ),
        (
            "src/global.php".to_owned(),
            String::from_utf8_lossy(global_source).into_owned(),
        ),
        (
            "src/Worker.php".to_owned(),
            String::from_utf8_lossy(worker_source).into_owned(),
        ),
    ]);
    let resolved = compass_resolve::resolve(
        &[helpers.clone(), global.clone(), worker.clone()],
        &sources,
    );
    let reversed = compass_resolve::resolve(&[worker, global, helpers], &sources);
    assert_eq!(universal_edges(&resolved), universal_edges(&reversed));

    for qualified in [
        "app\\logs::write",
        "app\\baseworker::parentwork",
        "app\\worker::localwork",
        "helpers\\assist",
        "globalhelp",
    ] {
        let target = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified)
            .ok_or_else(|| format!("missing target {qualified}: {:#?}", resolved.nodes))?;
        assert!(
            resolved.edges.iter().any(|edge| {
                edge.target == target.id
                    && edge.string("relation") == "calls"
                    && edge.string("language") == "php"
            }),
            "missing call to {qualified}: {:#?}",
            resolved.edges
        );
    }
    let worker = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app\\worker")
        .ok_or("missing Worker")?;
    assert!(
        resolved.edges.iter().any(|edge| {
            edge.target == worker.id
                && edge.string("relation") == "calls"
                && edge.string("language") == "php"
                && edge.string("rule").starts_with("universal-construction-")
        }),
        "worker={worker:#?} edges={:#?}",
        resolved.edges
    );
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("resolution_rule") == "php-global-function-fallback"
    }));
    Ok(())
}

#[test]
fn php_case_collisions_and_trait_conflicts_remain_ambiguous() {
    let left = extract(
        "src/left.php",
        b"<?php namespace Collision; function Render(): void {}",
    );
    let right = extract(
        "src/right.php",
        b"<?php namespace Collision; function RENDER(): void {}",
    );
    let caller = extract(
        "src/caller.php",
        br#"<?php
namespace Collision;
trait First { public function Work(): void {} }
trait Second { public function WORK(): void {} }
class Service { use First, Second; public function Run(): void { $this->work(); render(); } }
"#,
    );
    let sources = HashMap::from([
        (
            "src/left.php".to_owned(),
            "<?php namespace Collision; function Render(): void {}".to_owned(),
        ),
        (
            "src/right.php".to_owned(),
            "<?php namespace Collision; function RENDER(): void {}".to_owned(),
        ),
        (
            "src/caller.php".to_owned(),
            "<?php namespace Collision; trait First { public function Work(): void {} } trait Second { public function WORK(): void {} } class Service { use First, Second; public function Run(): void { $this->work(); render(); } }".to_owned(),
        ),
    ]);
    let resolved = compass_resolve::resolve(&[left, right, caller], &sources);
    let run_id = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "collision\\service::run")
        .map(|node| node.id.as_str());
    assert!(run_id.is_some(), "missing collision\\service::run");
    assert!(resolved.edges.iter().all(|edge| {
        !(run_id.is_some_and(|run_id| edge.source == run_id)
            && edge.string("relation") == "calls"
            && resolved.nodes.iter().any(|node| {
                node.id == edge.target
                    && matches!(
                        node.string("qualified_name").as_str(),
                        "collision\\render"
                            | "collision\\first::work"
                            | "collision\\second::work"
                    )
            }))
    }));
}
