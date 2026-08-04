use std::fs;
use std::path::Path;

use compass_languages::Engine;
use compass_resolve::frameworks::{
    FrameworkQualificationCase, FrameworkQualificationError, FrameworkRouteExpectation,
    qualify_framework_case,
};

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/code-graph/routes/typescript/express.ts")
}

fn typescript_fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/code-graph/routes/typescript")
        .join(name)
}

#[test]
fn qualification_cases_share_exact_route_and_handler_contracts()
-> Result<(), Box<dyn std::error::Error>> {
    let path = fixture();
    let extraction = Engine::default().extract(&path)?;
    let case = FrameworkQualificationCase::new(
        "express-basic",
        vec![FrameworkRouteExpectation::new("express", "GET", "/health").with_handler("health")],
    );
    let report = qualify_framework_case(
        &extraction,
        compass_languages::FrameworkLimits::default(),
        &case,
    )?;
    assert_eq!(report.case_id, "express-basic");
    assert_eq!(report.expected_routes, 1);
    assert_eq!(report.matched_routes, 1);
    Ok(())
}

#[test]
fn qualification_rejects_unresolved_or_missing_framework_routes()
-> Result<(), Box<dyn std::error::Error>> {
    let path = fixture();
    let source = fs::read(&path)?;
    let extraction = Engine::default().extract_source(Path::new("routes.ts"), &source)?;
    let case = FrameworkQualificationCase::new(
        "missing-route",
        vec![FrameworkRouteExpectation::new("express", "GET", "/missing")],
    );
    let error = qualify_framework_case(
        &extraction,
        compass_languages::FrameworkLimits::default(),
        &case,
    )
    .expect_err("a missing framework route must fail qualification");
    assert!(matches!(
        error,
        FrameworkQualificationError::MissingRoute { .. }
    ));
    Ok(())
}

#[test]
fn qualification_cases_cover_fastify_hono_and_remix_adapters()
-> Result<(), Box<dyn std::error::Error>> {
    for (name, case) in [
        (
            "fastify.ts",
            FrameworkQualificationCase::new(
                "fastify-health",
                vec![
                    FrameworkRouteExpectation::new("fastify", "GET", "/health")
                        .with_handler("health"),
                ],
            ),
        ),
        (
            "hono.ts",
            FrameworkQualificationCase::new(
                "hono-health",
                vec![
                    FrameworkRouteExpectation::new("hono", "GET", "/health").with_handler("health"),
                ],
            ),
        ),
        (
            "remix/app/routes/users.$id.tsx",
            FrameworkQualificationCase::new(
                "remix-user",
                vec![
                    FrameworkRouteExpectation::new("remix", "LOADER", "/users/{id}")
                        .with_handler("loader"),
                    FrameworkRouteExpectation::new("remix", "PAGE", "/users/{id}")
                        .with_handler("User"),
                ],
            ),
        ),
    ] {
        let extraction = Engine::default().extract(&typescript_fixture(name))?;
        let report = qualify_framework_case(
            &extraction,
            compass_languages::FrameworkLimits::default(),
            &case,
        )?;
        assert_eq!(report.expected_routes, report.matched_routes, "{name}");
    }
    Ok(())
}
