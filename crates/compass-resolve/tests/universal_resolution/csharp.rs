#[test]
fn csharp_cross_file_calls_overloads_and_overrides_use_the_shared_index()
-> Result<(), Box<dyn std::error::Error>> {
    let service_source = br#"namespace Demo;
public class BaseService { public virtual int Run(string value) => 1; }
public class Service : BaseService {
    public override int Run(string value) => 2;
    public int Find(string value) => 1;
    public int Find(string value, int limit) => 2;
}
"#;
    let caller_source = br#"namespace Demo;
public class Caller {
    public int Execute(Service service) {
        service.Find("one");
        service.Find("two", 2);
        return service.Run("value");
    }
}
"#;
    let service = extract("src/Service.cs", service_source);
    let caller = extract("src/Caller.cs", caller_source);
    let sources = HashMap::from([
        (
            "src/Service.cs".to_owned(),
            String::from_utf8_lossy(service_source).into_owned(),
        ),
        (
            "src/Caller.cs".to_owned(),
            String::from_utf8_lossy(caller_source).into_owned(),
        ),
    ]);
    let resolved = compass_resolve::resolve(&[caller.clone(), service.clone()], &sources);
    let reversed = compass_resolve::resolve(&[service, caller], &sources);
    assert_eq!(universal_edges(&resolved), universal_edges(&reversed));

    let finds = resolved
        .nodes
        .iter()
        .filter(|node| node.string("qualified_name") == "Demo.Service::Find")
        .collect::<Vec<_>>();
    assert_eq!(finds.len(), 2, "nodes={:#?}", resolved.nodes);
    assert_eq!(
        finds
            .iter()
            .map(|node| node.string("overload_discriminator"))
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["overload:0".to_owned(), "overload:1".to_owned()])
    );
    for find in &finds {
        assert!(resolved.edges.iter().any(|edge| {
            edge.target == find.id
                && edge.string("relation") == "calls"
                && edge.string("language") == "csharp"
        }), "find={find:#?} edges={:#?}", resolved.edges);
    }
    let base_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "Demo.BaseService::Run")
        .ok_or("missing base Run")?;
    let override_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "Demo.Service::Run")
        .ok_or("missing override Run")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == override_run.id
            && edge.target == base_run.id
            && edge.string("relation") == "overrides"
    }));
    Ok(())
}

#[test]
fn csharp_same_named_cross_namespace_targets_remain_ambiguous()
-> Result<(), Box<dyn std::error::Error>> {
    let left = extract(
        "src/Left.cs",
        b"namespace Left; public class Worker { public void Run() {} }",
    );
    let right = extract(
        "src/Right.cs",
        b"namespace Right; public class Worker { public void Run() {} }",
    );
    let caller = extract(
        "src/Caller.cs",
        b"namespace App; public class Caller { public void Execute(Worker worker) { worker.Run(); } }",
    );
    let sources = HashMap::from([
        (
            "src/Left.cs".to_owned(),
            "namespace Left; public class Worker { public void Run() {} }".to_owned(),
        ),
        (
            "src/Right.cs".to_owned(),
            "namespace Right; public class Worker { public void Run() {} }".to_owned(),
        ),
        (
            "src/Caller.cs".to_owned(),
            "namespace App; public class Caller { public void Execute(Worker worker) { worker.Run(); } }".to_owned(),
        ),
    ]);
    let resolved = compass_resolve::resolve(&[left, right, caller], &sources);
    let caller = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "App.Caller::Execute")
        .ok_or("missing caller")?;
    assert!(resolved.edges.iter().all(|edge| {
        !(edge.source == caller.id
            && edge.string("relation") == "calls"
            && resolved.nodes.iter().any(|node| {
                node.id == edge.target
                    && matches!(
                        node.string("qualified_name").as_str(),
                        "Left.Worker::Run" | "Right.Worker::Run"
                    )
            }))
    }));
    Ok(())
}

#[test]
fn csharp_bases_resolve_by_namespace_and_using_without_crossing_languages()
-> Result<(), Box<dyn std::error::Error>> {
    let base = extract(
        "src/Base.cs",
        b"namespace Demo.Core; public class BaseService { public void Run() {} }",
    );
    let contract = extract(
        "src/IWorker.cs",
        b"namespace Contracts; public interface IWorker { void Work(); }",
    );
    let implementation = extract(
        "src/Worker.cs",
        br#"using Contracts;
namespace Demo.Core;
public class Worker : BaseService, IWorker { public void Work() {} }
"#,
    );
    let java_collision = extract(
        "src/Base.java",
        b"package Demo.Core; public class BaseService {}",
    );
    let sources = HashMap::from([
        (
            "src/Base.cs".to_owned(),
            "namespace Demo.Core; public class BaseService { public void Run() {} }".to_owned(),
        ),
        (
            "src/IWorker.cs".to_owned(),
            "namespace Contracts; public interface IWorker { void Work(); }".to_owned(),
        ),
        (
            "src/Worker.cs".to_owned(),
            "using Contracts; namespace Demo.Core; public class Worker : BaseService, IWorker { public void Work() {} }".to_owned(),
        ),
        (
            "src/Base.java".to_owned(),
            "package Demo.Core; public class BaseService {}".to_owned(),
        ),
    ]);
    let resolved = compass_resolve::resolve(
        &[implementation, java_collision, contract, base],
        &sources,
    );
    let worker = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "Demo.Core.Worker")
        .ok_or("missing Worker")?;
    let base = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("qualified_name") == "Demo.Core.BaseService"
                && node.string("language") == "csharp"
        })
        .ok_or("missing C# BaseService")?;
    let contract = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "Contracts.IWorker")
        .ok_or("missing IWorker")?;
    assert!(
        resolved.edges.iter().any(|edge| {
            edge.source == worker.id
                && edge.target == base.id
                && edge.string("relation") == "inherits"
        }),
        "edges={:#?}",
        resolved.edges
    );
    assert!(
        resolved.edges.iter().any(|edge| {
            edge.source == worker.id
                && edge.target == contract.id
                && edge.string("relation") == "implements"
        }),
        "edges={:#?}",
        resolved.edges
    );
    assert!(resolved.edges.iter().all(|edge| {
        edge.source != worker.id
            || edge.string("relation") != "inherits"
            || resolved.nodes.iter().all(|node| {
                node.id != edge.target || node.string("language") != "java"
            })
    }));
    Ok(())
}

#[test]
fn csharp_typed_receivers_cover_members_generics_interfaces_and_inheritance()
-> Result<(), Box<dyn std::error::Error>> {
    let contracts = extract(
        "src/IWorker.cs",
        b"namespace Contracts; public interface IWorker { void Work(); }",
    );
    let services = extract(
        "src/Services.cs",
        br#"using Contracts;
namespace Services;
public class BaseService { public void Run() {} }
public class Worker : BaseService, IWorker { public void Work() {} }
"#,
    );
    let caller_source = br#"using Contracts;
using Services;
namespace App;
public class Caller {
    private Worker field;
    public Worker Property { get; set; }
    public void Execute(Worker parameter, IWorker contract) {
        Worker local = new Worker();
        parameter.Run();
        local.Run();
        field.Run();
        Property.Run();
        contract.Work();
    }
    public void ExecuteGeneric<T>(T contract) where T : IWorker { contract.Work(); }
}
"#;
    let caller = extract("src/Caller.cs", caller_source);
    let sources = HashMap::from([
        (
            "src/IWorker.cs".to_owned(),
            "namespace Contracts; public interface IWorker { void Work(); }".to_owned(),
        ),
        (
            "src/Services.cs".to_owned(),
            "using Contracts; namespace Services; public class BaseService { public void Run() {} } public class Worker : BaseService, IWorker { public void Work() {} }".to_owned(),
        ),
        (
            "src/Caller.cs".to_owned(),
            String::from_utf8_lossy(caller_source).into_owned(),
        ),
    ]);
    let resolved = compass_resolve::resolve(&[caller, contracts, services], &sources);
    let run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "Services.BaseService::Run")
        .ok_or("missing inherited Run")?;
    let work = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "Contracts.IWorker::Work")
        .ok_or("missing interface Work")?;
    let run_calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.target == run.id && edge.string("relation") == "calls")
        .count();
    let work_calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.target == work.id && edge.string("relation") == "calls")
        .count();
    assert_eq!(run_calls, 4, "edges={:#?}", resolved.edges);
    assert_eq!(work_calls, 2, "edges={:#?}", resolved.edges);
    Ok(())
}
