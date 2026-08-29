# Design

One bounded integration harness creates isolated home/config directories,
exports each package with the built Compass binary, invokes the installed
harness validator/installer, verifies discovery and MCP registration, performs
an upgrade, and uninstalls. A user-authored sentinel outside each managed
package must survive. Logs are redacted and bounded; no credentials or remote
model calls are permitted.

The qualification consumes installed artifacts, never source templates. CI
runs it after the normal phase-end integration build and the OpenCode
workspace JavaScript gates.
