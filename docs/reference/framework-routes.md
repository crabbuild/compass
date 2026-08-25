# How Compass maps framework routes

Compass detects supported framework registrations during local structural extraction and publishes them in `compass.graph/1`. Detection stays local: route packs do not download grammars, invoke language servers, or require credentials. This reference lists the route shapes Compass recognizes, the graph records it emits, and the cases it leaves unresolved.

> **Who this page is for:** Compass users and integrators who need to trace application entry points.
>
> **You will learn:** how route nodes and `routes_to` edges work, which framework forms Compass recognizes, and why some dynamic routes remain unresolved.
>
> **Prerequisites:** [Graph model](../concepts/graph-model.md).
>
> **Reading time:** about 8 minutes.

## Route records and edges

Compass records a route when it can connect a framework registration to source evidence. The record keeps the path, operation, declaration, and resolution state together so you can inspect the result without guessing.

Each registration becomes a `route` node with these details:

- **Framework and operation**: the framework family and HTTP, page, hook, messaging, or subscription operation
- **Path**: the normalized path plus the original path expression
- **Declaration**: the source scope and anchor that declared the route
- **Stages**: ordered framework stages, including middleware, dependency,
  security, loader/action, layout/boundary, and handler roles
- **Resolution**: the exact target, bounded candidates, or an explicit unresolved state

A `routes_to` edge points from the route to each exact stage target. Compass
preserves declaration order and repeated registrations. `dependency` identifies
a dependency-injection provider, while `security` identifies an authorization
or authentication provider; neither value contributes to `middlewareCount`.
The terminal handler remains last when a framework supplies one. Compass does
not create an exact edge when several targets remain valid.

`dependency` and `security` are additive values in the strict Code Graph v1,
query, task-context, CLI/MCP, and viewer contracts. Strict consumers must accept
those exact spellings and continue rejecting unknown values; readers built
against the older closed stage list must fail rather than silently translating
them to middleware.

Configuration and file-convention routes keep the rule and source that produced them. Equivalent inputs receive deterministic route identities and ordering.

## Read a route through callers

The `callers` command follows incoming `calls` and `routes_to` edges. Use it to see code callers and URL registrations for the same handler:

```bash
compass callers UsersController.show --graph compass-out/graph.json
```

The output stays tied to source anchors. Open the listed file and line when you need to verify a registration or investigate an unresolved target.

## Supported route shapes

Compass activates a framework pack only when the repository contains direct evidence, such as an import, exact receiver, framework configuration file, or dependency-backed file convention. The lists below describe the supported registration shapes.

### Python web frameworks

- **Django (`django-python` v1)**: exact imported `path`, `re_path`, legacy `url`, and `include` calls that contribute to `urlpatterns`; positional or named `route` and `view` arguments; function views, dotted string handlers, and class-based `.as_view()` handlers. Same-named local helpers and calls outside `urlpatterns` do not activate the pack.
- **Flask (`flask-python` v1)**: exact `Flask` and `Blueprint` receiver declarations and route decorators, constructor and registration-time `url_prefix`, positional or named `rule`, and literal `methods` lists. A `route` with no declared methods records `GET`; implicit HEAD/OPTIONS behavior is not published as additional routes.
- **FastAPI (`fastapi-python` v2)**: exact `FastAPI` and `APIRouter` receiver declarations; HTTP and WebSocket decorators; `api_route`, `route`, `add_api_route`, and `add_api_websocket_route`; literal paths and method lists; and constructor plus `include_router` prefixes. Application, router, include, route, parameter-default, and `Annotated` `Depends`/`Security` evidence becomes ordered dependency/security stages. Exact subdependencies use `depends_on`, and `yield` providers retain lifecycle detail without executing Python.
- **Starlette (`starlette-python` v1)**: exact `Starlette` and `Router` receivers; `route` and `websocket_route` decorators; imperative `add_route` and `add_websocket_route`; `Route` and `WebSocketRoute` constructors; inline and receiver-backed `Mount` composition; literal paths and method lists. Dynamic endpoints, paths, and mounts remain unresolved.
- **Pydantic (`pydantic-python` v1)**: exact `BaseModel` subclasses and source-proven model inheritance receive the existing `model` role. Existing field, validator, serializer, and computed-field declarations are retained rather than replaced with synthetic schema nodes. FastAPI parameter/return annotations and exact literal `response_model` values publish `depends_on` edges from handlers to request/response models.

FastAPI routers, Starlette applications/routers, and Flask blueprints compose through a bounded receiver-
identity multigraph. Repeated mounts retain their source anchors and
multiplicity, nested prefixes compose outer-to-inner, cycles publish no
invented route, and an over-depth traversal is an explicit limit error.
Computed paths/prefixes, rebound receivers, wrong-framework imports, and
ambiguous receiver identities remain unresolved. These Python packs are
fixture-qualified but remain `Qualifying` until the pinned independent audit
meets the production thresholds.

FastAPI/Starlette middleware, lifespan, and background-task registrations are
not published as route or dependency edges by these pack versions. Their
registration meaning needs an independently reviewed relation-capability
contract; same-named or dynamic calls are not approximated in the meantime.

### JavaScript and TypeScript frameworks

- **Express**: `express()` and `Router()` receivers; `get`, `post`, `put`, `patch`, `delete`, `options`, `head`, and `all`; literal paths, ordered middleware chains, and literal `use(prefix, importedRouter)` mounts across modules; opaque inline callbacks remain unresolved
- **Fastify**: `fastify()` receivers; HTTP method calls and `route({ method, url, handler })` objects; literal `register(importedPlugin, { prefix })` mounts across modules, ordered hook stages such as `preHandler`, and opaque inline callbacks remain unresolved
- **Hono**: `new Hono()` receivers; HTTP method calls, `on([methods], path, handler)` arrays, `basePath` chains, and literal `route(prefix, child)` mounts within and across modules with ordered middleware stages
- **Angular Router**: typed `Routes` arrays and arrays passed to `provideRouter`, `RouterModule.forRoot`, `RouterModule.forChild`, or `resetConfig`; nested literal paths; component targets; and opaque lazy `loadComponent` or `loadChildren` targets
- **Next.js**: App Router `page.*` and `route.*` files under `app` or `src/app`, Pages Router pages and `pages/api` handlers, dynamic segments, route groups, named HTTP exports, and project activation from the `next` dependency or `next.config.*`
- **Remix**: flat and nested route modules under `app/routes`, `routes`, or `src/routes`; `_index`, dotted nested segments, `$param` and splat names; and `PAGE`, `LOADER`, and `ACTION` operations from default, loader, and action exports. Project activation uses `@remix-run/*` dependencies or `remix.config.*`.
- **NestJS**: `Controller` HTTP method decorators and `RequestMapping`; GraphQL `Resolver` `Query` and `Mutation` operations at `/graphql`; typed GraphQL field details; `WebSocketGateway` `SubscribeMessage`; `MessagePattern` and `EventPattern` transport registrations
- **React Router**: JSX `Route` elements with `element={<Component />}` or `Component={Component}`, plus literal object route configs with `component`, `element`, or `Component` targets and loader or action stages
- **SvelteKit**: `src/routes` `+page.svelte` components and `+server` endpoints, including `[param]` and `[...rest]` segments and source-backed exported HTTP methods; `+page.ts` load modules are not pages
- **Vue Router and Nuxt**: Vue Router literal route objects; Nuxt `pages` components; `server/api` method-suffixed endpoints; dynamic segments; and route-middleware domain facts
- **Astro**: `src/pages` `.astro` pages and `.ts` or `.js` endpoints, including `[param]` and `[...rest]` segments, exported HTTP methods, `ALL`, and source-backed default handlers

Vite configuration is represented as a `configuration` node rather than a
route. The node retains bounded `resolve.alias`, plugin imports, and
configuration keys so build-time wiring remains inspectable without inventing
HTTP endpoints.

### PHP, Ruby, and JVM frameworks

- **Laravel**: exact `Illuminate\\Support\\Facades\\Route` receivers, including aliases; HTTP, `match`, `any`, `prefix(...)->group`, `resource`, `apiResource`, and `only` or `except` resource modifiers; `Controller@action` and controller or action tuple handlers
- **Drupal**: `*.routing.yml` and `*.routing.yaml` paths with controller, form, entity-form, entity-view, or entity-list handlers; pipe or comma-separated `_method` values; documented `hook_*` implementations and matching functions in `.module`, `.theme`, `.install`, and `.inc` files
- **Rails**: HTTP and `match` declarations inside `Rails.application.routes.draw`, `to:` and hash-rocket handlers, literal `scope` and `namespace` prefixes with namespaced controller owners, and `via` method lists
- **Spring**: Java and Kotlin controller mappings, including class and method composition, HTTP mapping annotations, `RequestMapping` methods, composed and inherited Java mappings, constants, packages, and overloaded handler signatures
- **Play**: literal `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `OPTIONS`, and `HEAD` entries in `conf/routes`, targeting Java, Scala, or injected controller actions

### Go, Rust, and native server frameworks

- **Gin, Echo, Fiber, chi, and gorilla/mux**: imported router registrations, grouped or closure prefixes, ordered Gin/Fiber middleware chains, Echo and Fiber HTTP method calls, chi method calls, and gorilla `HandleFunc(...).Methods(...)`, including `PathPrefix(...).Subrouter()` chains
- **Axum, actix-web, and Rocket**: Axum literal `nest` and `merge` composition
  across local router variables and uniquely resolved module factory calls,
  state-wrapped handlers, ordered `layer` and `route_layer` middleware, and
  route registrations;
  actix scoped resources and `.route(...).to(...)` handlers; plus Rocket and
  actix route attributes, including multiline attributes, guarded by framework
  imports or qualified macros. Ambiguous or cyclic Axum module factory targets
  remain local and uncomposed.
- **Vapor**: grouped literal and path-component prefixes, closure groups, `app.on(...)`, and HTTP registrations with explicit `use:` handlers; opaque closures remain visible as unresolved handlers
- **ASP.NET Core**: universal C# evidence-backed MVC controller and action templates, `[controller]` and `[action]` tokens, aliased HTTP method attributes, `AcceptVerbs`, `[NonAction]`, and absolute `/` or `~/` action-template overrides; Minimal API `MapGet`, `MapPost`, `MapPut`, `MapPatch`, `MapDelete`, `MapOptions`, `MapHead`, and literal `MapMethods` registrations; nested literal `MapGroup` prefixes; named handlers; and source-anchored multiline lambdas

## Special route contracts

Some frameworks expose routes that are not HTTP URLs. Compass keeps their domain meaning instead of assigning a misleading path:

- NestJS messaging and WebSocket subscriptions retain their typed messaging or subscription contracts
- Nuxt middleware retains its middleware domain fact
- GraphQL fields retain the field name in route details while sharing the `/graphql` transport endpoint

These records still retain handler references and source anchors. File-route packs require matching project dependency or framework-configuration evidence during a normal repository build. Direct single-file extraction remains available for fixtures and tooling, but it does not activate a repository pack by itself.

## Why a route can remain unresolved

Compass publishes a route only when its evidence identifies a framework and a target within bounded limits. The following forms do not become exact route bindings:

- computed or concatenated paths
- dynamic method names and arbitrary metaprogramming
- opaque closures with no source-backed callable
- same-named handlers with more than one valid target
- targets selected only by repository-wide terminal-name similarity
- file-route conventions without matching project dependency evidence

An unresolved route remains visible on its `route` node. Compass keeps bounded candidates when available and omits a misleading `routes_to` edge. Limit errors remain explicit diagnostics, never empty successful results. Django include cycles stop without inventing a route.

## A practical route investigation workflow

Use this sequence when you need to explain how a request reaches a handler:

1. Run `compass update . --no-viz` to publish a local graph snapshot
2. Run `compass callers UsersController.show --graph compass-out/graph.json` to list code callers and route registrations
3. Open each source anchor and compare the declaration with the route path, stages, and resolution state
4. Treat a missing `routes_to` edge as unresolved evidence, not as proof that the route has no handler

## Related pages

- [Graph model](../concepts/graph-model.md)
- [Provenance](../concepts/provenance.md)
- [Outputs](outputs.md)

**Next step:** build a graph, then run `compass callers` for a known controller or view.
