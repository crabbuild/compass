# Security

Report vulnerabilities through
[GitHub private vulnerability reporting](https://github.com/crabbuild/compass/security/advisories/new).

The extension runs Compass only in trusted workspaces, invokes no shell,
authorizes source paths against the selected repository, loads no remote
webview assets, emits no telemetry, and stores temporary historical exports in
extension-owned private storage. Initialization enumerates at most 5,001 local
workspace files, publishes only normalized repository-contained relative paths
to its webview, and caps submitted include and exclude rules at 256 each.
