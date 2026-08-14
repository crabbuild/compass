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
