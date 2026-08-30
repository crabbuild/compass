use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

use compass_languages::{Engine, RawFrameworkFact};
use compass_resolve::resolve;

fn extract_java(path: &str, source: &str) -> Result<compass_languages::Extraction, Box<dyn Error>> {
    Ok(Engine::default().extract_source(Path::new(path), source.as_bytes())?)
}

#[test]
fn spring_java_routes_are_derived_from_universal_annotation_evidence() -> Result<(), Box<dyn Error>>
{
    let extraction = extract_java(
        "src/main/java/example/UserController.java",
        r#"
package example;
import org.springframework.web.bind.annotation.RequestMapping;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RestController;

@RestController
@RequestMapping(path = {"/api", "/internal"})
class UserController {
    @GetMapping(path = {"/users", "/people"})
    String list() { return "ok"; }
}
"#,
    )?;
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Annotation(annotation)
            if annotation.pack_id == "spring-java"
                && annotation.annotation_name == "GetMapping")
    }));
    assert!(
        !extraction
            .framework_facts
            .iter()
            .any(|fact| { matches!(fact, RawFrameworkFact::Route(_)) })
    );

    let resolved = resolve(&[extraction], &HashMap::new());
    let routes = resolved
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Route(route) => {
                Some((route.operation.as_str(), route.normalized_path.as_str()))
            }
            RawFrameworkFact::Domain(_)
            | RawFrameworkFact::Annotation(_)
            | RawFrameworkFact::Role(_)
            | RawFrameworkFact::Relation(_)
            | RawFrameworkFact::Configuration(_)
            | RawFrameworkFact::FileSet(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        routes,
        [
            ("GET", "/api/users"),
            ("GET", "/api/people"),
            ("GET", "/internal/users"),
            ("GET", "/internal/people"),
        ]
    );
    assert!(
        resolved
            .edges
            .iter()
            .any(|edge| edge.string("relation") == "routes_to")
    );
    assert!(
        resolved
            .edges
            .iter()
            .any(|edge| edge.string("relation") == "registers")
    );
    Ok(())
}

#[test]
fn spring_components_register_and_single_constructors_inject_exact_types()
-> Result<(), Box<dyn Error>> {
    let repository = extract_java(
        "src/main/java/example/UserRepository.java",
        r#"
package example;
import org.springframework.stereotype.Repository;
@Repository class UserRepository {}
"#,
    )?;
    let service = extract_java(
        "src/main/java/example/UserService.java",
        r#"
package example;
import org.springframework.stereotype.Service;
@Service class UserService {
    private final UserRepository repository;
    UserService(UserRepository repository) { this.repository = repository; }
}
"#,
    )?;
    let composed = extract_java(
        "src/main/java/example/UseCase.java",
        r#"
package example;
import org.springframework.stereotype.Service;
@Service @interface UseCase {}
@UseCase class BillingService {}
"#,
    )?;
    let resolved = resolve(&[repository, service, composed], &HashMap::new());
    assert_eq!(
        resolved
            .edges
            .iter()
            .filter(|edge| edge.string("relation") == "registers")
            .count(),
        3
    );
    assert!(resolved.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Domain(domain)
            if domain.kind == "bean_definition" && domain.name == "billingService")
    }));
    let dependency = resolved
        .edges
        .iter()
        .find(|edge| edge.string("relation") == "depends_on")
        .ok_or("missing constructor injection")?;
    let source = resolved
        .nodes
        .iter()
        .find(|node| node.id == dependency.source)
        .ok_or("missing dependency source")?;
    let target = resolved
        .nodes
        .iter()
        .find(|node| node.id == dependency.target)
        .ok_or("missing dependency target")?;
    assert_eq!(source.string("qualified_name"), "example.UserService");
    assert_eq!(target.string("qualified_name"), "example.UserRepository");
    Ok(())
}

#[test]
fn spring_composed_and_interface_mappings_target_the_implementation() -> Result<(), Box<dyn Error>>
{
    let api = extract_java(
        "src/main/java/example/Api.java",
        r#"
package example;
import java.lang.annotation.ElementType;
import java.lang.annotation.Target;
import org.springframework.core.annotation.AliasFor;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RequestMapping;

@Target(ElementType.METHOD)
@GetMapping("/default")
@interface Read {
    @AliasFor(annotation = GetMapping.class, attribute = "path")
    String[] value() default {};
}

@RequestMapping("/api")
interface Api {
    @Read("/users") String users();
}
"#,
    )?;
    let implementation = extract_java(
        "src/main/java/example/ApiController.java",
        r#"
package example;
import org.springframework.web.bind.annotation.RestController;
@RestController class ApiController implements Api {
    public String users() { return "ok"; }
}
"#,
    )?;
    assert!(
        api.framework_facts.iter().any(|fact| {
            matches!(fact, RawFrameworkFact::Annotation(annotation)
            if annotation.annotation_name == "AliasFor"
                && annotation.detail.get("annotationAttribute").and_then(serde_json::Value::as_str)
                    == Some("value"))
        }),
        "missing alias evidence: {:#?}",
        api.framework_facts
    );
    let resolved = resolve(&[api, implementation], &HashMap::new());
    let inherited = resolved
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Route(route)
                if route.rule.as_deref() == Some("spring-inherited-request-mapping") =>
            {
                Some(route)
            }
            _ => None,
        })
        .find(|route| route.normalized_path == "/api/users")
        .ok_or_else(|| {
            format!(
                "missing inherited composed route: {:#?}",
                resolved.framework_facts
            )
        })?;
    let target = resolved
        .nodes
        .iter()
        .find(|node| node.id == inherited.handler_reference)
        .ok_or("missing inherited handler")?;
    assert_eq!(
        target.string("qualified_name"),
        "example.ApiController::users"
    );
    Ok(())
}

#[test]
fn spring_messaging_producers_and_consumers_publish_typed_domain_edges()
-> Result<(), Box<dyn Error>> {
    let extraction = extract_java(
        "src/main/java/example/Messaging.java",
        r#"
package example;
import org.springframework.stereotype.Service;
import org.springframework.kafka.annotation.KafkaListener;
import org.springframework.kafka.core.KafkaTemplate;
@Service class Messaging {
    private final KafkaTemplate<String, String> kafka;
    Messaging(KafkaTemplate<String, String> kafka) { this.kafka = kafka; }
    @KafkaListener(topics = {"orders", "audit"})
    void consume(String value) {}
    void produce(String value) { kafka.send("orders", value); }
}
"#,
    )?;
    let resolved = resolve(&[extraction], &HashMap::new());
    for relation in ["consumes", "produces"] {
        assert!(
            resolved
                .edges
                .iter()
                .any(|edge| edge.string("relation") == relation),
            "missing {relation}: {:#?}",
            resolved.framework_facts
        );
    }
    Ok(())
}

#[test]
fn spring_route_constants_resolve_across_static_imports_and_concatenation()
-> Result<(), Box<dyn Error>> {
    let paths = extract_java(
        "src/main/java/example/Paths.java",
        r#"
package example;
import org.springframework.web.bind.annotation.RequestMapping;
final class Paths {
    static final String ROOT = "/api";
    static final String USERS = ROOT + "/users";
}
"#,
    )?;
    let controller = extract_java(
        "src/main/java/example/ConstantController.java",
        r#"
package example;
import static example.Paths.USERS;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RestController;
@RestController class ConstantController {
    @GetMapping(USERS) String users() { return "ok"; }
}
"#,
    )?;
    assert!(
        paths.framework_facts.iter().any(|fact| {
            matches!(fact, RawFrameworkFact::Domain(domain) if domain.kind == "_spring_constant")
        }),
        "missing constant evidence: {:#?}",
        paths.framework_facts
    );
    let resolved = resolve(&[paths, controller], &HashMap::new());
    assert!(
        resolved.framework_facts.iter().any(|fact| {
            matches!(fact, RawFrameworkFact::Route(route) if route.normalized_path == "/api/users")
        }),
        "missing constant route: {:#?}",
        resolved.framework_facts
    );
    assert!(!resolved.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Domain(domain) if domain.kind == "_spring_constant")
    }));
    Ok(())
}

#[test]
fn spring_stereotypes_data_scheduling_transactions_and_security_stay_typed()
-> Result<(), Box<dyn Error>> {
    let model = extract_java(
        "src/main/java/example/User.java",
        r#"
package example;
import jakarta.persistence.Entity;
import jakarta.persistence.Table;
@Entity @Table(name = "users", schema = "app") class User {}
"#,
    )?;
    let repository = extract_java(
        "src/main/java/example/UserRepository.java",
        r#"
package example;
import org.springframework.data.jpa.repository.JpaRepository;
interface UserRepository extends JpaRepository<User, Long> {}
"#,
    )?;
    let service = extract_java(
        "src/main/java/example/SecuredService.java",
        r#"
package example;
import org.springframework.scheduling.annotation.Scheduled;
import org.springframework.security.access.prepost.PreAuthorize;
import org.springframework.stereotype.Service;
import org.springframework.transaction.annotation.Transactional;
@Service class SecuredService {
    @Scheduled(cron = "0 * * * * *")
    @Transactional
    @PreAuthorize("hasRole('ADMIN')")
    void reconcile() {}
}
"#,
    )?;
    let resolved = resolve(&[model, repository, service], &HashMap::new());
    assert!(resolved.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Domain(domain)
            if domain.kind == "orm_mapping"
                && domain.detail.get("database_table").and_then(|value| value.as_str()) == Some("users"))
    }));
    assert!(
        resolved.framework_facts.iter().any(|fact| {
            matches!(fact, RawFrameworkFact::Domain(domain)
            if domain.kind == "bean_definition" && domain.name == "userRepository")
        }),
        "missing repository bean: {:#?}",
        resolved.framework_facts
    );
    for relation in ["registers", "schedules", "triggers"] {
        assert!(
            resolved
                .edges
                .iter()
                .any(|edge| edge.string("relation") == relation)
        );
    }
    let method = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "example.SecuredService::reconcile")
        .ok_or("missing secured scheduled method")?;
    let traits = method
        .attributes
        .get("framework_traits")
        .and_then(|value| value.as_array())
        .ok_or("missing framework traits")?;
    for expected in ["secured", "transactional"] {
        assert!(traits.iter().any(|value| value.as_str() == Some(expected)));
    }
    Ok(())
}
